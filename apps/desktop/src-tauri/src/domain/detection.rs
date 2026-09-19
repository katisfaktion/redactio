use super::{
    mapping::{recover_pending, Mapping},
    scan::{scan_collection, ScanReport},
    settings::{load_settings, save_settings, ProcessingConfig, Settings, SyncPair},
    sync::{configure_pair, fingerprint, OperationGuard, RunController},
};
use crate::{
    error::AppError,
    protocol::{ConfigureResult, Detection, PreviewRulesPayload, PreviewRulesResult},
    sidecar::Sidecar,
};
use std::{path::Path, time::Duration};
use uuid::Uuid;

// Interactive synthetic previews should fail promptly; document processing keeps its 120s limit.
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(5);

struct ResetOnDrop<'a> {
    sidecar: &'a Sidecar,
    armed: bool,
}
impl Drop for ResetOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.sidecar.discard_configuration();
        }
    }
}

/// Concrete shared preparation for activation, scan, batch, review and export.
/// Caller retains app→config→source guards. Configure twice before writing: the
/// proposed snapshot first, then the final opaque revision. Failed validation
/// never changes settings, and no-op saves never rewrite the file.
pub(crate) async fn apply_configuration(
    path: &Path,
    guard: &OperationGuard,
    pair: &mut SyncPair,
    config: ProcessingConfig,
    sidecar: &Sidecar,
) -> Result<ConfigureResult, AppError> {
    let mut reset = ResetOnDrop {
        sidecar,
        armed: true,
    };
    let saved = pair.clone();
    let result = async {
        guard
            .config
            .validate(path.parent().ok_or_else(|| AppError::new("invalid_path"))?)?;
        guard.source()?.validate(&pair.source_folder)?;
        let mut settings = load_settings(path)?;
        let saved_settings = settings.clone();
        if !settings.sync_pairs.iter().any(|current| current == pair) {
            return Err(AppError::new("configuration_changed"));
        }
        let mut proposed = pair.clone();
        proposed.config = config;
        proposed.processing_revision = Uuid::new_v4();
        let validated = configure_pair(&proposed, sidecar).await?;
        let changed = settings.apply_validated_config(
            pair.id,
            proposed.config,
            fingerprint(&validated.engine),
        )?;
        let resolved = settings
            .sync_pairs
            .iter()
            .find(|current| current.id == pair.id)
            .unwrap()
            .clone();
        let configured = configure_pair(&resolved, sidecar).await?;
        if fingerprint(&configured.engine) != fingerprint(&validated.engine) {
            return Err(AppError::new("processing_version_changed"));
        }
        if changed {
            let mut mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder)?;
            recover_pending(pair, &mut mapping, guard.source()?, path, &guard.config)?;
            if mapping
                .entries()
                .iter()
                .any(|entry| entry.pending.is_some())
            {
                return Err(AppError::new("recovery_pending"));
            }
        }
        guard.source()?.validate(&pair.source_folder)?;
        guard.config.validate(path.parent().unwrap())?;
        if load_settings(path)? != saved_settings {
            return Err(AppError::new("configuration_changed"));
        }
        settings.validate_registry(&[path.parent().unwrap().to_path_buf()])?;
        if changed {
            save_settings(path, &settings)?;
        }
        *pair = resolved;
        Ok(configured)
    }
    .await;
    if result.is_ok() {
        reset.armed = false;
    } else {
        // Atomic replacement may have succeeded before a durability error. Restore
        // actual disk truth, never an assumed old revision after such an error.
        if let Ok(settings) = load_settings(path) {
            if let Some(actual) = settings.sync_pairs.iter().find(|p| {
                p.id == saved.id
                    && p.source_folder == saved.source_folder
                    && p.target_folder == saved.target_folder
            }) {
                if configure_pair(actual, sidecar).await.is_ok() {
                    reset.armed = false;
                }
            }
        }
    }
    result
}

pub async fn save_processing_config(
    controller: &RunController,
    sidecar: &Sidecar,
    pair_id: Uuid,
    config: ProcessingConfig,
) -> Result<Settings, AppError> {
    let (mut pair, guard) = controller.lock_pair(pair_id)?;
    apply_configuration(
        controller.settings_path(),
        &guard,
        &mut pair,
        config,
        sidecar,
    )
    .await?;
    load_settings(controller.settings_path())
}

pub async fn refresh_processing_config(
    controller: &RunController,
    sidecar: &Sidecar,
    pair_id: Uuid,
) -> Result<Settings, AppError> {
    let (mut pair, guard) = controller.lock_pair(pair_id)?;
    let config = pair.config.clone();
    apply_configuration(
        controller.settings_path(),
        &guard,
        &mut pair,
        config,
        sidecar,
    )
    .await?;
    load_settings(controller.settings_path())
}

pub async fn scan_configured(
    controller: &RunController,
    sidecar: &Sidecar,
    pair_id: Uuid,
) -> Result<ScanReport, AppError> {
    let (mut pair, guard) = controller.lock_pair(pair_id)?;
    let config = pair.config.clone();
    apply_configuration(
        controller.settings_path(),
        &guard,
        &mut pair,
        config,
        sidecar,
    )
    .await?;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        scan_collection(&pair)
    })
    .await
    .map_err(|_| AppError::new("scan_unavailable"))?
}

pub async fn preview_rules(
    controller: &RunController,
    sidecar: &Sidecar,
    pair_id: Uuid,
    config: ProcessingConfig,
    text: String,
) -> Result<Vec<Detection>, AppError> {
    let (saved, _guard) = controller.lock_pair(pair_id)?;
    let mut reset = ResetOnDrop {
        sidecar,
        armed: true,
    };
    let mut proposed = saved.clone();
    proposed.config = config;
    proposed.processing_revision = Uuid::new_v4();
    let result = async {
        configure_pair(&proposed, sidecar).await?;
        let preview: PreviewRulesResult = sidecar
            .request(
                "preview_rules",
                &PreviewRulesPayload {
                    sync_pair_id: pair_id,
                    processing_revision: proposed.processing_revision,
                    text: text.clone(),
                },
                PREVIEW_TIMEOUT,
            )
            .await?;
        let length = text.chars().count() as u64;
        if preview.detections.iter().any(|d| d.end > length) {
            return Err(AppError::new("invalid_sidecar_response"));
        }
        Ok(preview.detections)
    }
    .await;
    let restored = configure_pair(&saved, sidecar).await;
    if restored.is_ok() {
        reset.armed = false;
    }
    match (result, restored) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(detections), Ok(_)) => Ok(detections),
    }
}
