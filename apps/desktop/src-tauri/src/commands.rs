use crate::sidecar::Sidecar;
use crate::{
    domain::{
        detection,
        mapping::CollectionGuard,
        paths::{create_target as create_target_directory, validate_roots},
        recovery::{fresh_start, recovery_pairs, RecoveryPair},
        review::{self, ReviewViewData, SaveReview},
        scan::ScanReport,
        settings::{load_settings, save_settings, ProcessingConfig, Settings},
        sync::{RunController, RunSummary},
    },
    error::AppError,
};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

pub struct AppState {
    settings_path: PathBuf,
    app_config_root: PathBuf,
    runs: RunController,
    sidecar: Mutex<Option<Sidecar>>,
    models: Mutex<Option<crate::model_manager::ModelManager>>,
}

impl AppState {
    fn models(&self) -> Result<crate::model_manager::ModelManager, AppError> {
        let mut models = self
            .models
            .lock()
            .map_err(|_| AppError::new("state_unavailable"))?;
        if models.is_none() {
            *models = Some(crate::model_manager::ModelManager::new(
                crate::resources::resolve()?,
            ));
        }
        Ok(models.as_ref().unwrap().clone())
    }
    fn cached_sidecar(&self) -> Option<Sidecar> {
        self.sidecar.lock().ok().and_then(|saved| saved.clone())
    }
    pub async fn shutdown(&self) {
        let models = self.models.lock().ok().and_then(|models| models.clone());
        if let Some(models) = models {
            models.shutdown().await;
        }
        self.shutdown_sidecar().await;
    }

    fn sidecar(&self) -> Result<Sidecar, AppError> {
        let mut saved = self
            .sidecar
            .lock()
            .map_err(|_| AppError::new("state_unavailable"))?;
        if saved.is_none() {
            let resources = crate::resources::resolve()?;
            *saved = Some(Sidecar::new(
                resources.sidecar_executable,
                resources.sidecar_args,
                resources.model_root,
            ));
        }
        Ok(saved.as_ref().unwrap().clone())
    }

    pub fn initialize(app: &AppHandle) -> Result<Self, AppError> {
        let app_config_root = app
            .path()
            .app_config_dir()
            .map_err(|_| AppError::new("app_config_unavailable"))?;
        fs::create_dir_all(&app_config_root)?;
        Ok(Self {
            settings_path: app_config_root.join("settings.json"),
            app_config_root: app_config_root.clone(),
            runs: RunController::new(app_config_root.join("settings.json")),
            sidecar: Mutex::new(None),
            models: Mutex::new(None),
        })
    }

    pub async fn shutdown_sidecar(&self) {
        let sidecar = self.sidecar.lock().ok().and_then(|saved| saved.clone());
        if let Some(sidecar) = sidecar {
            sidecar.shutdown().await;
        }
    }
}

#[derive(Serialize)]
pub struct UiSyncPair {
    id: Uuid,
    name: String,
    source_folder: String,
    target_folder: String,
    created_at: String,
    processing_revision: Uuid,
    config: ProcessingConfig,
}

#[derive(Serialize)]
pub struct UiSettings {
    schema_version: u8,
    sync_pairs: Vec<UiSyncPair>,
    selected_sync_pair_id: Option<Uuid>,
}

impl From<Settings> for UiSettings {
    fn from(settings: Settings) -> Self {
        Self {
            schema_version: settings.schema_version,
            selected_sync_pair_id: settings.selected_sync_pair_id,
            sync_pairs: settings
                .sync_pairs
                .into_iter()
                .map(|pair| UiSyncPair {
                    id: pair.id,
                    name: pair.name,
                    source_folder: pair.source_folder.to_string_lossy().into_owned(),
                    target_folder: pair.target_folder.to_string_lossy().into_owned(),
                    created_at: pair.created_at,
                    processing_revision: pair.processing_revision,
                    config: pair.config,
                })
                .collect(),
        }
    }
}

#[tauri::command]
pub fn list_pairs(state: State<'_, AppState>) -> Result<UiSettings, AppError> {
    let _guard = state.runs.try_operation()?;
    Ok(load_registry(&state)?.into())
}

#[tauri::command]
pub fn list_recovery_pairs(state: State<'_, AppState>) -> Result<Vec<RecoveryPair>, AppError> {
    let _guard = state.runs.try_operation()?;
    let _config_guard = CollectionGuard::acquire(&state.app_config_root)?;
    recovery_pairs(&state.settings_path)
}

#[tauri::command]
pub fn fresh_start_pair(
    state: State<'_, AppState>,
    pair_id: Uuid,
    target_folder: String,
    confirmed: bool,
) -> Result<(), AppError> {
    let _guard = state.runs.try_operation()?;
    fresh_start(
        &state.settings_path,
        pair_id,
        Path::new(&target_folder),
        confirmed,
    )?;
    Ok(())
}

#[tauri::command]
pub fn add_pair(
    state: State<'_, AppState>,
    name: String,
    source_folder: String,
    target_folder: String,
    create_target: bool,
    model_name: String,
) -> Result<UiSettings, AppError> {
    mutate(&state, |settings| {
        let resources = crate::resources::resolve()?;
        let model = ready_model(
            crate::resources::list_models(&resources.model_root)?,
            &model_name,
        )?;
        let _lease =
            crate::model_store::ModelUseGuard::acquire(&resources.model_root, &model.name)?;
        let source = Path::new(&source_folder);
        let target = Path::new(&target_folder);
        let mut other_roots = settings.roots();
        other_roots.push(state.app_config_root.clone());
        match fs::metadata(target) {
            Ok(_) => {
                validate_roots(source, target, &other_roots)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_target_directory(source, target, &other_roots, create_target)?;
            }
            Err(error) => return Err(error.into()),
        }
        let pair_id = settings.add(&name, source, target)?;
        let pair = settings
            .sync_pairs
            .iter_mut()
            .find(|pair| pair.id == pair_id)
            .unwrap();
        pair.config = native_default_config(&model);
        Ok(())
    })
}

#[tauri::command]
pub fn rename_pair(
    state: State<'_, AppState>,
    pair_id: Uuid,
    name: String,
) -> Result<UiSettings, AppError> {
    mutate(&state, |settings| settings.rename(pair_id, &name))
}

#[tauri::command]
pub fn select_pair(state: State<'_, AppState>, pair_id: Uuid) -> Result<UiSettings, AppError> {
    mutate(&state, |settings| settings.select(pair_id))
}

#[tauri::command]
pub fn remove_pair(state: State<'_, AppState>, pair_id: Uuid) -> Result<UiSettings, AppError> {
    mutate(&state, |settings| settings.remove(pair_id))
}

#[tauri::command]
pub async fn scan_pair(state: State<'_, AppState>, pair_id: Uuid) -> Result<ScanReport, AppError> {
    detection::scan_configured(&state.runs, &state.sidecar()?, pair_id).await
}

#[tauri::command]
pub async fn save_processing_config(
    state: State<'_, AppState>,
    pair_id: Uuid,
    config: ProcessingConfig,
) -> Result<UiSettings, AppError> {
    validate_model_config(&config)?;
    Ok(
        detection::save_processing_config(&state.runs, &state.sidecar()?, pair_id, config)
            .await?
            .into(),
    )
}

#[tauri::command]
pub async fn refresh_processing_config(
    state: State<'_, AppState>,
    pair_id: Uuid,
) -> Result<UiSettings, AppError> {
    Ok(
        detection::refresh_processing_config(&state.runs, &state.sidecar()?, pair_id)
            .await?
            .into(),
    )
}

#[tauri::command]
pub async fn preview_rules(
    state: State<'_, AppState>,
    pair_id: Uuid,
    config: ProcessingConfig,
    text: String,
) -> Result<Vec<crate::protocol::Detection>, AppError> {
    validate_model_config(&config)?;
    detection::preview_rules(&state.runs, &state.sidecar()?, pair_id, config, text).await
}

#[tauri::command]
pub fn list_models() -> Result<Vec<crate::protocol::ModelInfo>, AppError> {
    crate::resources::list_models(&crate::resources::resolve()?.model_root)
}

#[tauri::command]
pub async fn open_review(
    state: State<'_, AppState>,
    key: crate::protocol::DocumentKey,
) -> Result<ReviewViewData, AppError> {
    review::open_configured(&state.runs, &state.sidecar()?, &key).await
}

// Tauri exposes these named fields as the strict frontend save contract.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn save_review(
    state: State<'_, AppState>,
    key: crate::protocol::DocumentKey,
    expected_output_hash: String,
    expected_review_hash: String,
    decisions: crate::protocol::Decisions,
    status: crate::protocol::ReviewStatus,
    notes: String,
    acknowledged_warnings: Vec<String>,
) -> Result<ReviewViewData, AppError> {
    let input = SaveReview {
        expected_output_hash,
        expected_review_hash,
        decisions,
        status,
        notes,
        acknowledged_warnings,
    };
    review::save_configured(&state.runs, &state.sidecar()?, &key, input).await
}

fn mutate(
    state: &AppState,
    change: impl FnOnce(&mut Settings) -> Result<(), AppError>,
) -> Result<UiSettings, AppError> {
    let _guard = state.runs.try_operation()?;
    let _config_guard = CollectionGuard::acquire(&state.app_config_root)?;
    let mut settings = load_registry(state)?;
    change(&mut settings)?;
    settings.validate_registry(std::slice::from_ref(&state.app_config_root))?;
    match save_settings(&state.settings_path, &settings) {
        Ok(()) => Ok(settings.into()),
        Err(error) => {
            let _authoritative = load_settings(&state.settings_path);
            Err(error)
        }
    }
}

fn validate_model_config(config: &ProcessingConfig) -> Result<(), AppError> {
    config.validate()?;
    if requires_native_selection(&config.model) && config.model_entities.is_none() {
        return Err(AppError::new("invalid_settings"));
    }
    let resources = crate::resources::resolve()?;
    validate_model_config_against(
        config,
        crate::resources::list_models(&resources.model_root)?,
    )
}

fn requires_native_selection(name: &str) -> bool {
    !matches!(
        name,
        "de_core_news_lg" | "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
    )
}

fn ready_model(
    models: Vec<crate::protocol::ModelInfo>,
    name: &str,
) -> Result<crate::protocol::ModelInfo, AppError> {
    models
        .into_iter()
        .find(|model| model.compatible && model.name == name)
        .ok_or_else(|| AppError::new("model_not_found"))
}

fn validate_model_config_against(
    config: &ProcessingConfig,
    models: Vec<crate::protocol::ModelInfo>,
) -> Result<(), AppError> {
    config.validate()?;
    let model = ready_model(models, &config.model)?;
    if requires_native_selection(&config.model) && config.model_entities.is_none() {
        return Err(AppError::new("invalid_settings"));
    }
    if let Some(selected) = &config.model_entities {
        let supported = model.entity_types.into_iter().collect::<HashSet<_>>();
        if selected.iter().any(|entity| !supported.contains(entity)) {
            return Err(AppError::new("invalid_settings"));
        }
    }
    Ok(())
}

fn native_default_config(model: &crate::protocol::ModelInfo) -> ProcessingConfig {
    let mut config = ProcessingConfig {
        model: model.name.clone(),
        ..ProcessingConfig::default()
    };
    config
        .enabled_entities
        .retain(|entity| entity.is_supplementary());
    config.model_entities = Some(model.entity_types.clone());
    config
}

fn load_registry(state: &AppState) -> Result<Settings, AppError> {
    let settings = load_settings(&state.settings_path)?;
    settings.validate_registry(std::slice::from_ref(&state.app_config_root))?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pair_defaults_to_all_native_labels_and_only_supplementary_recognizers() {
        let model = crate::protocol::ModelInfo {
            name: "OpenMed-PII-German-BiomedBERT-Large-340M-v1".into(),
            version: "synthetic".into(),
            compatible: true,
            entity_types: vec![
                crate::domain::settings::EntityType::parse("ALIEN_42".into()).unwrap(),
            ],
        };
        let config = native_default_config(&model);
        assert_eq!(config.model_entities, Some(model.entity_types));
        assert_eq!(config.enabled_entities.len(), 6);
        assert!(config
            .enabled_entities
            .iter()
            .all(crate::domain::settings::EntityType::is_supplementary));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn hugginglil_requires_an_explicit_native_label_selection() {
        let config = ProcessingConfig {
            model: "pii-sensitive-ner-german".into(),
            ..ProcessingConfig::default()
        };
        assert_eq!(
            validate_model_config(&config).unwrap_err().code,
            "invalid_settings"
        );
    }

    #[test]
    fn pair_model_selection_uses_only_the_requested_ready_name() {
        let models = vec![
            crate::protocol::ModelInfo {
                name: "ready-first".into(),
                version: "one".into(),
                compatible: true,
                entity_types: vec![
                    crate::domain::settings::EntityType::parse("PERSON".into()).unwrap()
                ],
            },
            crate::protocol::ModelInfo {
                name: "requested".into(),
                version: "two".into(),
                compatible: true,
                entity_types: vec![
                    crate::domain::settings::EntityType::parse("DATE".into()).unwrap()
                ],
            },
        ];
        let selected = ready_model(models.clone(), "requested").unwrap();
        assert_eq!(native_default_config(&selected).model, "requested");
        assert_eq!(
            ready_model(models, "missing").unwrap_err().code,
            "model_not_found"
        );
    }

    #[test]
    fn every_nonlegacy_model_requires_native_labels() {
        let model = crate::protocol::ModelInfo {
            name: "hf:fixture/model@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            version: "a".repeat(40),
            compatible: true,
            entity_types: vec![crate::domain::settings::EntityType::parse("PERSON".into()).unwrap()],
        };
        let config = ProcessingConfig {
            model: model.name.clone(),
            ..ProcessingConfig::default()
        };
        assert_eq!(
            validate_model_config_against(&config, vec![model])
                .unwrap_err()
                .code,
            "invalid_settings"
        );
    }

    #[test]
    fn concurrent_model_removal_blocks_pair_mutation_before_it_can_create() {
        let root = tempfile::tempdir().unwrap();
        let settings_path = root.path().join("settings.json");
        save_settings(&settings_path, &Settings::default()).unwrap();
        let state = AppState {
            settings_path: settings_path.clone(),
            app_config_root: root.path().to_path_buf(),
            runs: RunController::new(settings_path.clone()),
            sidecar: Mutex::new(None),
            models: Mutex::new(None),
        };
        let removal = state.runs.try_operation().unwrap();
        let error = mutate(&state, |_| panic!("pair mutation ran during model removal"))
            .err()
            .unwrap();
        assert_eq!(error.code, "operation_busy");
        assert!(load_settings(&settings_path).unwrap().sync_pairs.is_empty());
        drop(removal);
    }
}

#[tauri::command]
pub fn start_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    pair_id: Uuid,
    relative_paths: Option<Vec<String>>,
    force_doc_ids: Vec<String>,
) -> Result<Uuid, AppError> {
    let run = state.runs.prepare(pair_id, relative_paths, force_doc_ids)?;
    let run_id = run.run_id;
    let sidecar = state.sidecar();
    let controller = state.runs.clone();
    tauri::async_runtime::spawn(async move {
        controller
            .execute(run, sidecar, |progress| {
                let _ = app.emit("run-progress", progress);
            })
            .await;
    });
    Ok(run_id)
}

#[tauri::command]
pub async fn cancel_sync(
    state: State<'_, AppState>,
    pair_id: Uuid,
    run_id: Uuid,
) -> Result<(), AppError> {
    state.runs.cancel(pair_id, run_id).await
}

#[tauri::command]
pub fn get_run_summary(
    state: State<'_, AppState>,
    pair_id: Uuid,
    run_id: Uuid,
) -> Result<Option<RunSummary>, AppError> {
    state.runs.summary(pair_id, run_id)
}

#[tauri::command]
pub fn audit_location(state: State<'_, AppState>) -> String {
    state
        .app_config_root
        .join(crate::domain::audit::AUDIT_FILE)
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub async fn export_approved(
    state: State<'_, AppState>,
    pair_id: Uuid,
    doc_ids: Vec<String>,
    destination: Option<PathBuf>,
) -> Result<crate::domain::export::ExportSummary, AppError> {
    crate::domain::export::export_approved(
        &state.runs,
        pair_id,
        doc_ids,
        destination.as_deref(),
        || state.sidecar(),
    )
    .await
}

#[tauri::command]
pub fn open_audit_folder(state: State<'_, AppState>) -> Result<(), AppError> {
    let directory = &state.app_config_root;
    #[cfg(windows)]
    let executable = "explorer.exe";
    #[cfg(target_os = "macos")]
    let executable = "open";
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let executable = "xdg-open";
    let mut child = std::process::Command::new(executable)
        .arg(directory)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| AppError::new("folder_open_failed"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
pub fn list_managed_models(
    state: State<'_, AppState>,
) -> Result<Vec<crate::model_store::ManagedModel>, AppError> {
    state.models()?.list(&state.settings_path)
}
#[tauri::command]
pub async fn check_model(
    state: State<'_, AppState>,
    source: crate::model_store::ModelSource,
) -> Result<crate::model_store::CheckedModel, AppError> {
    state.models()?.check(source, &state.runs).await
}
#[tauri::command]
pub async fn start_model_install(
    app: AppHandle,
    state: State<'_, AppState>,
    plan_id: Uuid,
) -> Result<Uuid, AppError> {
    state
        .models()?
        .start_install(
            plan_id,
            &state.runs,
            &state.app_config_root,
            state.cached_sidecar(),
            move |job| {
                let _ = app.emit("model-progress", job);
            },
        )
        .await
}
#[tauri::command]
pub async fn remove_model(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Uuid, AppError> {
    state
        .models()?
        .remove(
            &name,
            &state.runs,
            &state.app_config_root,
            state.cached_sidecar(),
            move |job| {
                let _ = app.emit("model-progress", job);
            },
        )
        .await
}
#[tauri::command]
pub async fn cancel_model_job(state: State<'_, AppState>, job_id: Uuid) -> Result<(), AppError> {
    state.models()?.cancel(job_id).await
}
#[tauri::command]
pub fn get_model_job(
    state: State<'_, AppState>,
    job_id: Uuid,
) -> Result<Option<crate::model_store::ModelJob>, AppError> {
    state.models()?.get_job(job_id)
}
