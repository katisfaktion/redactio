use crate::{
    domain::{mapping::CollectionGuard, settings::load_settings, sync::RunController},
    error::AppError,
    model_store::{
        self, CheckedModel, ManagedModel, ManagedState, ModelDescriptor, ModelJob, ModelJobStage,
        ModelRecord, ModelSource, ModelState, UsedByPair,
    },
    resources::ResourcePaths,
    sidecar::{read_frame_limit, Sidecar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::Command,
    sync::watch,
};
use uuid::Uuid;

const MAX_MESSAGE: usize = 2 * 1024 * 1024;
#[derive(Clone)]
pub struct ModelManager {
    inner: Arc<Inner>,
}
struct Inner {
    resources: ResourcePaths,
    state: Mutex<Jobs>,
}
#[derive(Default)]
struct Jobs {
    plan: Option<(Uuid, ModelDescriptor)>,
    active: Option<(Uuid, watch::Sender<bool>, watch::Receiver<bool>)>,
    jobs: HashMap<Uuid, ModelJob>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    payload: serde_json::Value,
}

impl ModelManager {
    pub fn new(resources: ResourcePaths) -> Self {
        Self {
            inner: Arc::new(Inner {
                resources,
                state: Mutex::new(Jobs::default()),
            }),
        }
    }

    pub fn list(&self, settings_path: &Path) -> Result<Vec<ManagedModel>, AppError> {
        let settings = load_settings(settings_path)?;
        let mut models = model_store::list_managed(&self.inner.resources.model_root)?;
        for model in &mut models {
            model.used_by_pairs = settings
                .sync_pairs
                .iter()
                .filter(|pair| pair.config.model == model.name)
                .map(|pair| UsedByPair {
                    id: pair.id.to_string(),
                    name: pair.name.clone(),
                })
                .collect();
            if model.state == ManagedState::Invalid {
                model.error = Some(AppError::new("model_incompatible"));
            }
        }
        Ok(models)
    }

    pub async fn check(
        &self,
        source: ModelSource,
        runs: &RunController,
    ) -> Result<CheckedModel, AppError> {
        let _operation = runs.try_operation()?;
        let id = Uuid::new_v4();
        let (_cancel, mut cancellation) = watch::channel(false);
        let value = self
            .worker(id, "check_model", &source, &mut cancellation, |_| {
                Err(protocol())
            })
            .await?;
        let descriptor: ModelDescriptor = serde_json::from_value(value).map_err(|_| protocol())?;
        // The response must bind the explicitly checked source, including immutable receipts.
        match &source {
            ModelSource::Catalog { key } => {
                if !model_store::catalog_models()?
                    .iter()
                    .any(|entry| &entry.key == key && entry.descriptor == descriptor)
                {
                    return Err(protocol());
                }
            }
            ModelSource::Receipt { name } => {
                let registry = model_store::read_registry(&self.inner.resources.model_root)?;
                if !registry.models.iter().any(|record| {
                    &record.descriptor.name == name && matches_plan(&record.descriptor, &descriptor)
                }) {
                    return Err(protocol());
                }
            }
            ModelSource::Url { url } => {
                if url.trim_end_matches('/')
                    != format!("https://huggingface.co/{}", descriptor.repository)
                {
                    return Err(protocol());
                }
            }
        }
        let catalog = model_store::catalog_models()?
            .into_iter()
            .find(|entry| entry.descriptor.name == descriptor.name)
            .map(|entry| entry.key);
        let model = model_store::managed(&descriptor, catalog, ManagedState::Available, 0);
        self.inner.state.lock().map_err(|_| unavailable())?.plan = Some((id, descriptor));
        Ok(CheckedModel {
            plan_id: id.to_string(),
            model,
        })
    }

    pub async fn start_install<F>(
        &self,
        plan_id: Uuid,
        runs: &RunController,
        config_root: &Path,
        sidecar: Option<Sidecar>,
        emit: F,
    ) -> Result<Uuid, AppError>
    where
        F: Fn(ModelJob) + Send + Sync + 'static,
    {
        let operation = runs.try_operation()?;
        let config = CollectionGuard::acquire(config_root)?;
        let descriptor = self
            .inner
            .state
            .lock()
            .map_err(|_| unavailable())?
            .plan
            .as_ref()
            .filter(|(id, _)| *id == plan_id)
            .map(|(_, descriptor)| descriptor.clone())
            .ok_or_else(|| AppError::new("model_plan_expired"))?;
        if let Some(sidecar) = sidecar {
            sidecar.shutdown().await;
        }
        self.start(descriptor, false, operation, config, emit)
    }

    pub async fn remove<F>(
        &self,
        name: &str,
        runs: &RunController,
        config_root: &Path,
        sidecar: Option<Sidecar>,
        emit: F,
    ) -> Result<Uuid, AppError>
    where
        F: Fn(ModelJob) + Send + Sync + 'static,
    {
        let operation = runs.try_operation()?;
        let config = CollectionGuard::acquire(config_root)?;
        if load_settings(&config_root.join("settings.json"))?
            .sync_pairs
            .iter()
            .any(|pair| pair.config.model == name)
        {
            return Err(AppError::new("model_in_use"));
        }
        let descriptor = model_store::read_registry(&self.inner.resources.model_root)?
            .models
            .into_iter()
            .find(|record| {
                record.descriptor.name == name
                    && matches!(record.state, ModelState::Ready | ModelState::Removing)
            })
            .ok_or_else(|| AppError::new("model_not_found"))?
            .descriptor;
        if let Some(sidecar) = sidecar {
            sidecar.shutdown().await;
        }
        self.start(descriptor, true, operation, config, emit)
    }

    fn start<F>(
        &self,
        descriptor: ModelDescriptor,
        remove: bool,
        operation: tokio::sync::OwnedMutexGuard<()>,
        config: CollectionGuard,
        emit: F,
    ) -> Result<Uuid, AppError>
    where
        F: Fn(ModelJob) + Send + Sync + 'static,
    {
        let id = Uuid::new_v4();
        let job = ModelJob {
            job_id: id.to_string(),
            model_name: descriptor.name.clone(),
            stage: if remove {
                ModelJobStage::Removing
            } else {
                ModelJobStage::Downloading
            },
            downloaded_bytes: 0,
            total_bytes: if remove {
                0
            } else {
                descriptor.files.iter().map(|file| file.size).sum()
            },
            error: None,
        };
        let (cancel, mut cancellation) = watch::channel(false);
        let (complete, completed) = watch::channel(false);
        {
            let mut state = self.inner.state.lock().map_err(|_| unavailable())?;
            if state.active.is_some() {
                return Err(AppError::new("operation_busy"));
            }
            state.jobs.insert(id, job.clone());
            state.active = Some((id, cancel, completed));
        }
        let manager = self.clone();
        tokio::spawn(async move {
            let mut job = job;
            let payload = if remove {
                serde_json::json!(descriptor.name)
            } else {
                serde_json::json!(descriptor)
            };
            let result = manager
                .worker(
                    id,
                    if remove {
                        "remove_model"
                    } else {
                        "install_model"
                    },
                    &payload,
                    &mut cancellation,
                    |value| {
                        let progress: ModelJob =
                            serde_json::from_value(value).map_err(|_| protocol())?;
                        if progress.job_id != job.job_id
                            || progress.model_name != job.model_name
                            || progress.total_bytes != job.total_bytes
                            || progress.downloaded_bytes < job.downloaded_bytes
                            || progress.error.is_some()
                            || !(if remove {
                                matches!(
                                    progress.stage,
                                    ModelJobStage::Removing | ModelJobStage::Removed
                                )
                            } else {
                                matches!(
                                    progress.stage,
                                    ModelJobStage::Downloading
                                        | ModelJobStage::Validating
                                        | ModelJobStage::Ready
                                )
                            })
                            || (job.stage == ModelJobStage::Validating
                                && progress.stage == ModelJobStage::Downloading)
                        {
                            return Err(protocol());
                        }
                        // A worker terminal hint is not a verified outcome. Only result + registry can publish it.
                        if matches!(
                            progress.stage,
                            ModelJobStage::Ready | ModelJobStage::Removed
                        ) {
                            return Ok(());
                        }
                        job = progress;
                        manager.store(id, &job)?;
                        emit(job.clone());
                        Ok(())
                    },
                )
                .await;
            let outcome = manager.reconcile(&descriptor, remove, result);
            match outcome {
                Ok(stage) => {
                    job.stage = stage;
                    if job.stage == ModelJobStage::Ready {
                        job.downloaded_bytes = job.total_bytes;
                    }
                }
                Err(error) => {
                    job.stage = if error.code == "operation_cancelled" {
                        ModelJobStage::Cancelled
                    } else {
                        ModelJobStage::Failed
                    };
                    job.error = if job.stage == ModelJobStage::Cancelled {
                        None
                    } else {
                        Some(error)
                    };
                }
            }
            // Keep both admission guards until the worker is reaped and disk state is reconciled.
            let _ = manager.store(id, &job);
            drop(config);
            drop(operation);
            if let Ok(mut state) = manager.inner.state.lock() {
                state.active = None;
            }
            complete.send_replace(true);
            emit(job);
        });
        Ok(id)
    }

    fn reconcile(
        &self,
        descriptor: &ModelDescriptor,
        remove: bool,
        result: Result<serde_json::Value, AppError>,
    ) -> Result<ModelJobStage, AppError> {
        let registry = model_store::read_registry(&self.inner.resources.model_root)?;
        let record = registry
            .models
            .iter()
            .find(|record| record.descriptor.name == descriptor.name);
        let published = if remove {
            record.is_some_and(|record| {
                record.state == ModelState::Available
                    && record.path.is_none()
                    && matches_plan(descriptor, &record.descriptor)
            })
        } else {
            record.is_some_and(|record| {
                record.state == ModelState::Ready && matches_plan(descriptor, &record.descriptor)
            }) && model_store::list_managed(&self.inner.resources.model_root)?
                .iter()
                .any(|model| model.name == descriptor.name && model.state == ManagedState::Ready)
        };
        match result {
            Err(error) if error.code == "operation_cancelled" && published => (),
            Err(error) => return Err(error),
            Ok(value) if remove => {
                let verified: ModelRecord =
                    serde_json::from_value(value).map_err(|_| protocol())?;
                if !published || record != Some(&verified) {
                    return Err(protocol());
                }
            }
            Ok(value) => {
                let verified: ModelDescriptor =
                    serde_json::from_value(value).map_err(|_| protocol())?;
                if !published || record.is_none_or(|record| record.descriptor != verified) {
                    return Err(protocol());
                }
            }
        }
        Ok(if remove {
            ModelJobStage::Removed
        } else {
            ModelJobStage::Ready
        })
    }

    fn store(&self, id: Uuid, job: &ModelJob) -> Result<(), AppError> {
        self.inner
            .state
            .lock()
            .map_err(|_| unavailable())?
            .jobs
            .insert(id, job.clone());
        Ok(())
    }
    pub fn get_job(&self, id: Uuid) -> Result<Option<ModelJob>, AppError> {
        Ok(self
            .inner
            .state
            .lock()
            .map_err(|_| unavailable())?
            .jobs
            .get(&id)
            .cloned())
    }
    pub async fn cancel(&self, id: Uuid) -> Result<(), AppError> {
        let completed = {
            let state = self.inner.state.lock().map_err(|_| unavailable())?;
            if let Some((active, cancel, completed)) = &state.active {
                if *active == id {
                    cancel.send_replace(true);
                    Some(completed.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(mut completed) = completed {
            while !*completed.borrow_and_update() {
                if completed.changed().await.is_err() {
                    break;
                }
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        let id = self
            .inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.active.as_ref().map(|(id, _, _)| *id));
        if let Some(id) = id {
            let _ = self.cancel(id).await;
        }
    }

    async fn worker<T: Serialize, F: FnMut(serde_json::Value) -> Result<(), AppError>>(
        &self,
        id: Uuid,
        kind: &str,
        payload: &T,
        cancellation: &mut watch::Receiver<bool>,
        mut progress: F,
    ) -> Result<serde_json::Value, AppError> {
        let mut bytes =
            serde_json::to_vec(&serde_json::json!({"id":id,"type":kind,"payload":payload}))
                .map_err(|_| protocol())?;
        if bytes.len() > MAX_MESSAGE {
            return Err(AppError::new("message_too_large"));
        }
        bytes.push(b'\n');
        let mut child = Command::new(&self.inner.resources.sidecar_executable)
            .args(&self.inner.resources.sidecar_args)
            .arg("--manage-models")
            .arg("--model-dir")
            .arg(&self.inner.resources.model_root)
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| AppError::new("model_worker_unavailable"))?;
        let mut stdin = child.stdin.take().ok_or_else(protocol)?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or_else(protocol)?);
        let exchange = async {
            stdin
                .write_all(&bytes)
                .await
                .map_err(|_| AppError::new("model_install_interrupted"))?;
            drop(stdin);
            let mut result = None;
            while let Some(line) = read_frame_limit(&mut stdout, MAX_MESSAGE).await? {
                let envelope: Envelope = serde_json::from_slice(&line).map_err(|_| protocol())?;
                if envelope.id != id.to_string() || result.is_some() {
                    return Err(protocol());
                }
                match envelope.kind.as_str() {
                    "progress" => progress(envelope.payload)?,
                    "result" => result = Some(envelope.payload),
                    "error" => {
                        return Err(serde_json::from_value::<AppError>(envelope.payload)
                            .map_err(|_| protocol())?)
                    }
                    _ => return Err(protocol()),
                }
            }
            result.ok_or_else(|| AppError::new("model_install_interrupted"))
        };
        use std::{future::Future, task::Poll};
        let mut exchange = std::pin::pin!(tokio::time::timeout(
            Duration::from_secs(if kind == "check_model" {
                180
            } else {
                24 * 60 * 60
            }),
            exchange
        ));
        let mut cancelled = std::pin::pin!(async {
            while !*cancellation.borrow_and_update() {
                if cancellation.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        });
        let result = std::future::poll_fn(|cx| {
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(AppError::new("operation_cancelled")));
            }
            match exchange.as_mut().poll(cx) {
                Poll::Ready(result) => Poll::Ready(
                    result.unwrap_or_else(|_| Err(AppError::new("model_network_timeout"))),
                ),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        if result.is_err() {
            let _ = child.start_kill();
        }
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        match status {
            Ok(Ok(status)) if status.success() || result.is_err() => result,
            _ => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(AppError::new("model_install_interrupted"))
            }
        }
    }
}
fn matches_plan(plan: &ModelDescriptor, result: &ModelDescriptor) -> bool {
    plan.name == result.name
        && plan.version == result.version
        && plan.repository == result.repository
        && plan.title == result.title
        && plan.license == result.license
        && plan.model_type == result.model_type
        && plan.architecture == result.architecture
        && plan.entity_types == result.entity_types
        && result.window_tokens <= plan.window_tokens
        && plan.files.len() == result.files.len()
        && plan.files.iter().zip(&result.files).all(|(left, right)| {
            left.filename == right.filename
                && left.size == right.size
                && left.upstream_hash == right.upstream_hash
                && left
                    .sha256
                    .as_ref()
                    .is_none_or(|hash| right.sha256.as_ref() == Some(hash))
        })
}
fn protocol() -> AppError {
    AppError::new("invalid_model_protocol")
}
fn unavailable() -> AppError {
    AppError::new("state_unavailable")
}
