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
}

impl AppState {
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
) -> Result<UiSettings, AppError> {
    mutate(&state, |settings| {
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
        settings.add(&name, source, target)?;
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

#[tauri::command]
pub async fn save_review(
    state: State<'_, AppState>,
    key: crate::protocol::DocumentKey,
    expected_output_hash: String,
    decisions: crate::protocol::Decisions,
    status: crate::protocol::ReviewStatus,
    notes: String,
    acknowledged_warnings: Vec<String>,
) -> Result<ReviewViewData, AppError> {
    let input = SaveReview {
        expected_output_hash,
        decisions,
        status,
        notes,
        acknowledged_warnings,
    };
    review::save_configured(&state.runs, &state.sidecar()?, &key, input).await
}

fn mutate(
    state: &State<'_, AppState>,
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

fn load_registry(state: &State<'_, AppState>) -> Result<Settings, AppError> {
    let settings = load_settings(&state.settings_path)?;
    settings.validate_registry(std::slice::from_ref(&state.app_config_root))?;
    Ok(settings)
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
