use crate::error::AppError;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::sync::Mutex;
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
    running: Mutex<Option<Arc<Running>>>,
    configured: Mutex<Option<SavedConfiguration>>,
    cancellation: AtomicU64,
}

#[derive(Clone)]
struct SavedConfiguration {
    payload: serde_json::Value,
    identity: (String, String),
}

struct Running {
    stdin: StdMutex<Option<ChildStdin>>,
    stdout: StdMutex<BufReader<std::process::ChildStdout>>,
    child: StdMutex<Option<Child>>,
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
                running: Mutex::new(None),
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
        if self.inner.cancellation.load(Ordering::SeqCst) != cancellation {
            return Err(AppError::new("operation_cancelled"));
        }
        let payload =
            serde_json::to_value(payload).map_err(|_| AppError::new("invalid_sidecar_request"))?;
        let identity = request_identity(kind, &payload)?;
        for attempt in 0..2 {
            let result = self
                .request_once(kind, &payload, identity.as_ref(), deadline)
                .await;
            if self.inner.cancellation.load(Ordering::SeqCst) != cancellation {
                return Err(AppError::new("operation_cancelled"));
            }
            match result {
                Ok(response) => {
                    if kind == "configure" {
                        let identity = identity
                            .clone()
                            .ok_or_else(|| AppError::new("invalid_sidecar_request"))?;
                        *self.inner.configured.lock().await = Some(SavedConfiguration {
                            payload: payload.clone(),
                            identity,
                        });
                    }
                    return serde_json::from_value(response)
                        .map_err(|_| AppError::new("invalid_sidecar_protocol"));
                }
                Err(AttemptError::Exited) if attempt == 0 => {
                    if matches!(kind, "process_document" | "preview_rules" | "render_review") {
                        let restored = self.restore_configuration(deadline).await;
                        if self.inner.cancellation.load(Ordering::SeqCst) != cancellation {
                            return Err(AppError::new("operation_cancelled"));
                        }
                        restored?;
                    }
                }
                Err(AttemptError::Exited) => return Err(AppError::new("engine_unavailable")),
                Err(AttemptError::Public(error)) => return Err(error),
            }
        }
        Err(AppError::new("engine_unavailable"))
    }

    async fn restore_configuration(&self, deadline: tokio::time::Instant) -> Result<(), AppError> {
        let saved = self.inner.configured.lock().await.clone();
        let Some(saved) = saved else {
            return Ok(());
        };
        match self
            .request_once("configure", &saved.payload, Some(&saved.identity), deadline)
            .await
        {
            Ok(_) => Ok(()),
            Err(AttemptError::Exited) => Err(AppError::new("engine_unavailable")),
            Err(AttemptError::Public(error)) => Err(error),
        }
    }

    async fn request_once(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        identity: Option<&(String, String)>,
        deadline: tokio::time::Instant,
    ) -> Result<serde_json::Value, AttemptError> {
        if tokio::time::Instant::now() >= deadline {
            return Err(AttemptError::Public(AppError::new("engine_timeout")));
        }
        let running = self.ensure_started().await.map_err(AttemptError::Public)?;
        let id = Uuid::new_v4().to_string();
        let mut bytes = serde_json::to_vec(&RequestEnvelope {
            id: &id,
            kind,
            payload,
        })
        .map_err(|_| AttemptError::Public(AppError::new("invalid_sidecar_request")))?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(AttemptError::Public(AppError::new("message_too_large")));
        }
        bytes.push(b'\n');

        let writer = running.clone();
        let write = tokio::task::spawn_blocking(move || {
            let mut stdin = writer.stdin.lock().map_err(|_| ())?;
            let stdin = stdin.as_mut().ok_or(())?;
            stdin
                .write_all(&bytes)
                .and_then(|()| stdin.flush())
                .map_err(|_| ())
        });
        match tokio::time::timeout_at(deadline, write).await {
            Ok(Ok(Ok(()))) => {}
            Ok(_) => {
                self.stop(&running).await;
                return Err(AttemptError::Exited);
            }
            Err(_) => {
                self.stop(&running).await;
                return Err(AttemptError::Public(AppError::new("engine_timeout")));
            }
        }

        let reader = running.clone();
        let expected_id = id.clone();
        let response = tokio::task::spawn_blocking(move || read_response(&reader, &expected_id));
        let envelope = match tokio::time::timeout_at(deadline, response).await {
            Ok(Ok(Ok(envelope))) => envelope,
            Ok(Ok(Err(error))) if error.code == "engine_unavailable" => {
                self.stop(&running).await;
                return Err(AttemptError::Exited);
            }
            Ok(Ok(Err(error))) => {
                self.stop(&running).await;
                return Err(AttemptError::Public(error));
            }
            Ok(Err(_)) => {
                self.stop(&running).await;
                return Err(AttemptError::Exited);
            }
            Err(_) => {
                self.stop(&running).await;
                return Err(AttemptError::Public(AppError::new("engine_timeout")));
            }
        };
        if envelope.kind == "error" {
            let error: AppError = serde_json::from_value(envelope.payload)
                .map_err(|_| AttemptError::Public(AppError::new("invalid_sidecar_protocol")))?;
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
        Ok(envelope.payload)
    }

    pub async fn shutdown(&self) {
        self.inner.cancellation.fetch_add(1, Ordering::SeqCst);
        let running = self.inner.running.lock().await.take();
        if let Some(running) = running {
            let _ = tokio::task::spawn_blocking(move || running.shutdown()).await;
        }
    }

    async fn ensure_started(&self) -> Result<Arc<Running>, AppError> {
        let mut state = self.inner.running.lock().await;
        if let Some(running) = state.as_ref() {
            return Ok(running.clone());
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
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|_| AppError::new("engine_unavailable"))?;
        let running = Arc::new(Running {
            stdin: StdMutex::new(child.stdin.take()),
            stdout: StdMutex::new(BufReader::new(
                child
                    .stdout
                    .take()
                    .ok_or_else(|| AppError::new("engine_unavailable"))?,
            )),
            child: StdMutex::new(Some(child)),
        });
        *state = Some(running.clone());
        Ok(running)
    }

    async fn stop(&self, running: &Arc<Running>) {
        let mut state = self.inner.running.lock().await;
        if state
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, running))
        {
            state.take();
        }
        running.terminate();
    }
}

fn read_response(running: &Running, expected_id: &str) -> Result<ResponseEnvelope, AppError> {
    let mut stdout = running
        .stdout
        .lock()
        .map_err(|_| AppError::new("engine_unavailable"))?;
    loop {
        let line = read_frame(&mut *stdout)?.ok_or_else(|| AppError::new("engine_unavailable"))?;
        let envelope = serde_json::from_slice::<ResponseEnvelope>(&line)
            .map_err(|_| AppError::new("invalid_sidecar_protocol"))?;
        if envelope.id == expected_id {
            return Ok(envelope);
        }
    }
}

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, AppError> {
    let mut frame = Vec::new();
    loop {
        let available = reader
            .fill_buf()
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

impl Running {
    fn shutdown(&self) {
        if let Ok(mut stdin) = self.stdin.lock() {
            stdin.take();
        }
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => return,
                        Ok(None) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        _ => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return;
                        }
                    }
                }
            }
        }
    }

    fn terminate(&self) {
        if let Ok(mut stdin) = self.stdin.lock() {
            stdin.take();
        }
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.terminate();
    }
}
