use crate::{error::AppError, protocol::ConfigureResult};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ffi::OsString,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::Mutex,
};
use uuid::Uuid;

const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
pub const INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(180);
pub const DOCUMENT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct Sidecar {
    inner: Arc<Inner>,
}

struct Inner {
    executable: PathBuf,
    args: Vec<OsString>,
    model_root: PathBuf,
    request_lock: Mutex<()>,
    running: StdMutex<Option<Arc<Running>>>,
    configured: Mutex<Option<SavedConfiguration>>,
    cancellation: AtomicU64,
}

#[derive(Clone)]
struct SavedConfiguration {
    payload: serde_json::Value,
    identity: (String, String),
}

struct Running {
    stdin: Mutex<Option<ChildStdin>>,
    stdout: Mutex<BufReader<tokio::process::ChildStdout>>,
    child: Mutex<Option<Child>>,
    configured_identity: StdMutex<Option<(String, String)>>,
}

struct AttemptGuard {
    inner: Arc<Inner>,
    running: Arc<Running>,
    armed: bool,
}

struct AttemptSuccess {
    payload: serde_json::Value,
    guard: AttemptGuard,
}

#[derive(Serialize)]
struct RequestEnvelope<'a, T> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    payload: &'a T,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseEnvelope {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    payload: serde_json::Value,
}

enum AttemptError {
    Exited,
    Public(AppError),
}

impl Sidecar {
    pub fn new(executable: PathBuf, args: Vec<OsString>, model_root: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                executable,
                args,
                model_root,
                request_lock: Mutex::new(()),
                running: StdMutex::new(None),
                configured: Mutex::new(None),
                cancellation: AtomicU64::new(0),
            }),
        }
    }

    pub async fn request<T: Serialize, R: DeserializeOwned>(
        &self,
        kind: &str,
        payload: &T,
        timeout: Duration,
    ) -> Result<R, AppError> {
        let cancellation = self.inner.cancellation.load(Ordering::SeqCst);
        let deadline = tokio::time::Instant::now() + timeout;
        let _request = tokio::time::timeout_at(deadline, self.inner.request_lock.lock())
            .await
            .map_err(|_| AppError::new("engine_timeout"))?;
        self.check_cancellation(cancellation)?;
        let payload =
            serde_json::to_value(payload).map_err(|_| AppError::new("invalid_sidecar_request"))?;
        let identity = request_identity(kind, &payload)?;

        for attempt in 0..2 {
            if is_collection_request(kind) {
                let restored = self.restore_configuration(deadline, cancellation).await;
                self.check_cancellation(cancellation)?;
                match restored {
                    Ok(()) => {}
                    Err(AttemptError::Exited) if attempt == 0 => continue,
                    Err(AttemptError::Exited) => {
                        return Err(AppError::new("engine_unavailable"));
                    }
                    Err(AttemptError::Public(error)) => return Err(error),
                }
            }
            let result = self
                .request_once(kind, &payload, identity.as_ref(), deadline, cancellation)
                .await;
            self.check_cancellation(cancellation)?;
            match result {
                Ok(success) => {
                    if kind == "configure" {
                        serde_json::from_value::<ConfigureResult>(success.payload.clone())
                            .map_err(|_| AppError::new("invalid_sidecar_protocol"))?;
                    }
                    let response = serde_json::from_value::<R>(success.payload.clone())
                        .map_err(|_| AppError::new("invalid_sidecar_protocol"))?;
                    if kind == "configure" {
                        let identity = identity
                            .clone()
                            .ok_or_else(|| AppError::new("invalid_sidecar_request"))?;
                        *self.inner.configured.lock().await = Some(SavedConfiguration {
                            payload: payload.clone(),
                            identity: identity.clone(),
                        });
                        success.guard.running.mark_configured(identity);
                    }
                    success.commit();
                    return Ok(response);
                }
                Err(AttemptError::Exited) if attempt == 0 => {}
                Err(AttemptError::Exited) => return Err(AppError::new("engine_unavailable")),
                Err(AttemptError::Public(error)) => return Err(error),
            }
        }
        Err(AppError::new("engine_unavailable"))
    }

    async fn restore_configuration(
        &self,
        deadline: tokio::time::Instant,
        cancellation: u64,
    ) -> Result<(), AttemptError> {
        let saved = self.inner.configured.lock().await.clone();
        let Some(saved) = saved else {
            return Ok(());
        };
        let running = self
            .ensure_started(cancellation)
            .map_err(AttemptError::Public)?;
        if running.is_configured(&saved.identity) {
            return Ok(());
        }
        let success = match self
            .request_once(
                "configure",
                &saved.payload,
                Some(&saved.identity),
                deadline,
                cancellation,
            )
            .await
        {
            Ok(success) => success,
            Err(error) => return Err(error),
        };
        self.check_cancellation(cancellation)
            .map_err(AttemptError::Public)?;
        serde_json::from_value::<ConfigureResult>(success.payload.clone())
            .map_err(|_| AttemptError::Public(AppError::new("invalid_sidecar_protocol")))?;
        success
            .guard
            .running
            .mark_configured(saved.identity.clone());
        success.commit();
        Ok(())
    }

    async fn request_once(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        identity: Option<&(String, String)>,
        deadline: tokio::time::Instant,
        cancellation: u64,
    ) -> Result<AttemptSuccess, AttemptError> {
        if tokio::time::Instant::now() >= deadline {
            return Err(AttemptError::Public(AppError::new("engine_timeout")));
        }
        let running = self
            .ensure_started(cancellation)
            .map_err(AttemptError::Public)?;
        let mut guard = AttemptGuard::new(self.inner.clone(), running.clone());
        let id = Uuid::new_v4().to_string();
        let mut bytes = serde_json::to_vec(&RequestEnvelope {
            id: &id,
            kind,
            payload,
        })
        .map_err(|_| AttemptError::Public(AppError::new("invalid_sidecar_request")))?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            guard.disarm();
            return Err(AttemptError::Public(AppError::new("message_too_large")));
        }
        bytes.push(b'\n');

        let write = async {
            let mut stdin = running.stdin.lock().await;
            let stdin = stdin
                .as_mut()
                .ok_or_else(|| AppError::new("engine_unavailable"))?;
            stdin
                .write_all(&bytes)
                .await
                .map_err(|_| AppError::new("engine_unavailable"))?;
            stdin
                .flush()
                .await
                .map_err(|_| AppError::new("engine_unavailable"))
        };
        match tokio::time::timeout_at(deadline, write).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                self.stop(&running).await;
                guard.disarm();
                return Err(AttemptError::Exited);
            }
            Err(_) => {
                self.stop(&running).await;
                guard.disarm();
                return Err(AttemptError::Public(AppError::new("engine_timeout")));
            }
        }

        let response = async {
            let mut stdout = running.stdout.lock().await;
            read_response(&mut stdout, &id).await
        };
        let envelope = match tokio::time::timeout_at(deadline, response).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(error)) if error.code == "engine_unavailable" => {
                self.stop(&running).await;
                guard.disarm();
                return Err(AttemptError::Exited);
            }
            Ok(Err(error)) => {
                self.stop(&running).await;
                guard.disarm();
                return Err(AttemptError::Public(error));
            }
            Err(_) => {
                self.stop(&running).await;
                guard.disarm();
                return Err(AttemptError::Public(AppError::new("engine_timeout")));
            }
        };
        if envelope.kind == "error" {
            let error: AppError = serde_json::from_value(envelope.payload)
                .map_err(|_| AttemptError::Public(AppError::new("invalid_sidecar_protocol")))?;
            guard.disarm();
            return Err(AttemptError::Public(error));
        }
        if envelope.kind != format!("{kind}_result") {
            return Err(AttemptError::Public(AppError::new(
                "invalid_sidecar_protocol",
            )));
        }
        if let Some((pair, revision)) = identity {
            let response_pair = envelope
                .payload
                .get("sync_pair_id")
                .and_then(serde_json::Value::as_str);
            let response_revision = envelope
                .payload
                .get("processing_revision")
                .and_then(serde_json::Value::as_str);
            if response_pair != Some(pair.as_str()) || response_revision != Some(revision.as_str())
            {
                return Err(AttemptError::Public(AppError::new(
                    "invalid_sidecar_protocol",
                )));
            }
        }
        Ok(AttemptSuccess {
            payload: envelope.payload,
            guard,
        })
    }

    pub async fn shutdown(&self) {
        self.inner.cancellation.fetch_add(1, Ordering::SeqCst);
        if let Some(running) = self.inner.take_running() {
            running.shutdown().await;
        }
    }

    fn check_cancellation(&self, expected: u64) -> Result<(), AppError> {
        if self.inner.cancellation.load(Ordering::SeqCst) == expected {
            Ok(())
        } else {
            Err(AppError::new("operation_cancelled"))
        }
    }

    fn ensure_started(&self, cancellation: u64) -> Result<Arc<Running>, AppError> {
        if let Some(running) = self.inner.current_running() {
            return Ok(running);
        }
        let mut command = Command::new(&self.inner.executable);
        command
            .args(&self.inner.args)
            .arg("--model-dir")
            .arg(&self.inner.model_root)
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|_| AppError::new("engine_unavailable"))?;
        let running = Arc::new(Running {
            stdin: Mutex::new(child.stdin.take()),
            stdout: Mutex::new(BufReader::new(
                child
                    .stdout
                    .take()
                    .ok_or_else(|| AppError::new("engine_unavailable"))?,
            )),
            child: Mutex::new(Some(child)),
            configured_identity: StdMutex::new(None),
        });
        let mut state = self
            .inner
            .running
            .lock()
            .map_err(|_| AppError::new("engine_unavailable"))?;
        if self.inner.cancellation.load(Ordering::SeqCst) != cancellation {
            running.abort_background();
            return Err(AppError::new("operation_cancelled"));
        }
        if let Some(current) = state.as_ref() {
            running.abort_background();
            return Ok(current.clone());
        }
        *state = Some(running.clone());
        Ok(running)
    }

    async fn stop(&self, running: &Arc<Running>) {
        self.inner.remove_running(running);
        running.kill_and_reap().await;
    }
}

impl AttemptSuccess {
    fn commit(mut self) {
        self.guard.disarm();
    }
}

impl AttemptGuard {
    fn new(inner: Arc<Inner>, running: Arc<Running>) -> Self {
        Self {
            inner,
            running,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        if self.armed {
            self.inner.remove_running(&self.running);
            self.running.abort_background();
        }
    }
}

impl Inner {
    fn current_running(&self) -> Option<Arc<Running>> {
        self.running.lock().ok()?.clone()
    }

    fn take_running(&self) -> Option<Arc<Running>> {
        self.running.lock().ok()?.take()
    }

    fn remove_running(&self, running: &Arc<Running>) {
        if let Ok(mut state) = self.running.lock() {
            if state
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, running))
            {
                state.take();
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(state) = self.running.get_mut() {
            if let Some(running) = state.take() {
                running.abort_background();
            }
        }
    }
}

async fn read_response(
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    expected_id: &str,
) -> Result<ResponseEnvelope, AppError> {
    loop {
        let line = read_frame(stdout)
            .await?
            .ok_or_else(|| AppError::new("engine_unavailable"))?;
        let envelope = serde_json::from_slice::<ResponseEnvelope>(&line)
            .map_err(|_| AppError::new("invalid_sidecar_protocol"))?;
        if envelope.id == expected_id {
            return Ok(envelope);
        }
    }
}

async fn read_frame(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<Vec<u8>>, AppError> {
    let mut frame = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|_| AppError::new("engine_unavailable"))?;
        if available.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(AppError::new("invalid_sidecar_protocol"))
            };
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if frame.len() + newline > MAX_MESSAGE_BYTES {
                return Err(AppError::new("message_too_large"));
            }
            frame.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            return Ok(Some(frame));
        }
        if frame.len() + available.len() > MAX_MESSAGE_BYTES {
            return Err(AppError::new("message_too_large"));
        }
        let consumed = available.len();
        frame.extend_from_slice(available);
        reader.consume(consumed);
    }
}

fn request_identity(
    kind: &str,
    payload: &serde_json::Value,
) -> Result<Option<(String, String)>, AppError> {
    if kind == "ping" {
        return Ok(None);
    }
    if !matches!(
        kind,
        "configure" | "process_document" | "preview_rules" | "render_review"
    ) {
        return Err(AppError::new("invalid_sidecar_request"));
    }
    let pair = payload
        .get("sync_pair_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::new("invalid_sidecar_request"))?;
    let revision = payload
        .get("processing_revision")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::new("invalid_sidecar_request"))?;
    if Uuid::parse_str(pair).is_err() || Uuid::parse_str(revision).is_err() {
        return Err(AppError::new("invalid_sidecar_request"));
    }
    Ok(Some((pair.into(), revision.into())))
}

fn is_collection_request(kind: &str) -> bool {
    matches!(kind, "process_document" | "preview_rules" | "render_review")
}

impl Running {
    fn is_configured(&self, identity: &(String, String)) -> bool {
        self.configured_identity
            .lock()
            .is_ok_and(|configured| configured.as_ref() == Some(identity))
    }

    fn mark_configured(&self, identity: (String, String)) {
        if let Ok(mut configured) = self.configured_identity.lock() {
            *configured = Some(identity);
        }
    }

    async fn shutdown(&self) {
        let stdin_closed = if let Ok(mut stdin) = self.stdin.try_lock() {
            stdin.take();
            true
        } else {
            false
        };
        let child = self.child.lock().await.take();
        let Some(mut child) = child else {
            return;
        };
        if stdin_closed
            && tokio::time::timeout(SHUTDOWN_GRACE, child.wait())
                .await
                .is_ok()
        {
            return;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }

    async fn kill_and_reap(&self) {
        if let Ok(mut stdin) = self.stdin.try_lock() {
            stdin.take();
        }
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    fn abort_background(&self) {
        let Ok(mut child_slot) = self.child.try_lock() else {
            return;
        };
        let Some(mut child) = child_slot.take() else {
            return;
        };
        let _ = child.start_kill();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.get_mut().take() {
            let _ = child.start_kill();
        }
    }
}
