use crate::{
    domain::{
        paths::{create_target as create_target_directory, validate_roots},
        settings::{load_settings, save_settings, ProcessingConfig, Settings},
    },
    error::AppError,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

pub struct AppState {
    settings_path: PathBuf,
    app_config_root: PathBuf,
    mutation: Mutex<()>,
}

impl AppState {
    pub fn initialize(app: &AppHandle) -> Result<Self, AppError> {
        let app_config_root = app
            .path()
            .app_config_dir()
            .map_err(|_| AppError::new("app_config_unavailable"))?;
        fs::create_dir_all(&app_config_root)?;
        Ok(Self {
            settings_path: app_config_root.join("settings.json"),
            app_config_root,
            mutation: Mutex::new(()),
        })
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
    let _guard = state
        .mutation
        .lock()
        .map_err(|_| AppError::new("state_unavailable"))?;
    Ok(load_registry(&state)?.into())
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

fn mutate(
    state: &State<'_, AppState>,
    change: impl FnOnce(&mut Settings) -> Result<(), AppError>,
) -> Result<UiSettings, AppError> {
    let _guard = state
        .mutation
        .lock()
        .map_err(|_| AppError::new("state_unavailable"))?;
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
