//! Explicit retirement of a collection whose allocation history is lost.
use super::{
    mapping::{CollectionGuard, MappingData, MAPPING_FILE},
    paths::{canonical_directory, reject_links, same_directory, validate_roots},
    scan::hash_output,
    settings::{load_settings, save_settings, Settings, SyncPair},
    storage::{open_validated_read, retain_private_metadata, ValidatedWrite},
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

const INTENT_FILE: &str = ".redactio-fresh-start.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Stage {
    Prepared,
    MetadataRetained,
    MappingPublished,
    Complete,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    schema_version: u8,
    old_pair: SyncPair,
    new_pair: SyncPair,
    settings_before_hash: String,
    settings_after_hash: String,
    old_mapping_hash: Option<String>,
    new_mapping_hash: String,
    private_metadata_present: bool,
    stage: Stage,
}

#[derive(Debug, Serialize)]
pub struct RecoveryPair {
    pub id: Uuid,
    pub name: String,
    pub source_folder: PathBuf,
    pub target_folder: PathBuf,
    pub pending_target: Option<PathBuf>,
}

fn bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec_pretty(value).map_err(|_| AppError::new("invalid_recovery"))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_optional(root: &Path, path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    let mut file = match open_validated_read(root, path) {
        Ok(file) => file,
        Err(error) if error.code == "path_unavailable" => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(AppError::new("invalid_recovery"));
    }
    Ok(Some(bytes))
}
fn read_intent(source: &Path) -> Result<Option<Intent>, AppError> {
    let Some(raw) = read_optional(source, &source.join(INTENT_FILE))? else {
        return Ok(None);
    };
    let intent: Intent =
        serde_json::from_slice(&raw).map_err(|_| AppError::new("invalid_recovery"))?;
    let settings = Settings {
        schema_version: 1,
        sync_pairs: vec![intent.old_pair.clone()],
        selected_sync_pair_id: Some(intent.old_pair.id),
    };
    settings.validate()?;
    let settings = Settings {
        sync_pairs: vec![intent.new_pair.clone()],
        selected_sync_pair_id: Some(intent.new_pair.id),
        ..settings
    };
    settings.validate()?;
    if !same_directory(&intent.old_pair.source_folder, source)?
        || intent.schema_version != 1
        || intent.old_pair.id == intent.new_pair.id
        || intent.old_pair.source_folder != intent.new_pair.source_folder
        || [
            &intent.settings_before_hash,
            &intent.settings_after_hash,
            &intent.new_mapping_hash,
        ]
        .into_iter()
        .any(|hash| !super::mapping::valid_hash(hash))
        || intent
            .old_mapping_hash
            .as_ref()
            .is_some_and(|hash| !super::mapping::valid_hash(hash))
    {
        return Err(AppError::new("invalid_recovery"));
    }
    Ok(Some(intent))
}
fn save_intent(source: &Path, intent: &Intent) -> Result<(), AppError> {
    ValidatedWrite::new(source, &source.join(INTENT_FILE))?.write_atomic(&bytes(intent)?)
}

pub fn ensure_recovered(source: &Path, pair_id: Uuid) -> Result<(), AppError> {
    if read_intent(source)?
        .is_some_and(|intent| intent.stage != Stage::Complete || intent.new_pair.id != pair_id)
    {
        return Err(AppError::new("mapping_recovery_required"));
    }
    Ok(())
}

/// Narrow startup recovery read: structural settings and root separation, never ordinary operations.
pub fn recovery_pairs(settings_path: &Path) -> Result<Vec<RecoveryPair>, AppError> {
    let settings = load_settings(settings_path)?;
    let config = settings_path
        .parent()
        .ok_or_else(|| AppError::new("invalid_path"))?;
    settings.validate_roots_with(&[config.to_path_buf()])?;
    let mut result = Vec::new();
    for pair in settings.sync_pairs {
        let intent = read_intent(&pair.source_folder)?;
        let pending = intent
            .as_ref()
            .is_some_and(|intent| intent.stage != Stage::Complete);
        let broken = MappingData::read(&pair.source_folder)
            .and_then(|data| data.validate_binding(pair.id, &pair.target_folder))
            .is_err();
        if pending || broken {
            result.push(RecoveryPair {
                id: pair.id,
                name: pair.name,
                source_folder: pair.source_folder,
                target_folder: pair.target_folder,
                pending_target: intent
                    .filter(|intent| intent.stage != Stage::Complete)
                    .map(|intent| intent.new_pair.target_folder),
            });
        }
    }
    Ok(result)
}

/// Requires explicit native confirmation. Holds config and source locks throughout.
/// Any failure returns immediately; the next deliberate retry reloads intent and disk truth.
pub fn fresh_start(
    settings_path: &Path,
    pair_id: Uuid,
    destination: &Path,
    confirmed: bool,
) -> Result<Settings, AppError> {
    fresh_start_with(settings_path, pair_id, destination, confirmed, |_| Ok(()))
}

fn fresh_start_with(
    settings_path: &Path,
    pair_id: Uuid,
    destination: &Path,
    confirmed: bool,
    mut after_stage: impl FnMut(Stage) -> Result<(), AppError>,
) -> Result<Settings, AppError> {
    if !confirmed {
        return Err(AppError::new("confirmation_required"));
    }
    let config = canonical_directory(
        settings_path
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?,
    )?;
    let _config_guard = CollectionGuard::acquire(&config)?;
    let mut settings = load_settings(settings_path)?;
    settings.validate_roots_with(std::slice::from_ref(&config))?;
    let index = settings
        .sync_pairs
        .iter()
        .position(|pair| pair.id == pair_id)
        .ok_or_else(|| AppError::new("unknown_pair"))?;
    let pair = settings.sync_pairs[index].clone();
    let source = canonical_directory(&pair.source_folder)?;
    let guard = CollectionGuard::acquire(&source)?;
    let saved_intent = read_intent(&source)?;
    let mut intent =
        if let Some(intent) = saved_intent.filter(|intent| intent.stage != Stage::Complete) {
            if pair != intent.old_pair && pair != intent.new_pair {
                return Err(AppError::new("invalid_recovery"));
            }
            if !same_directory(destination, &intent.new_pair.target_folder)? {
                return Err(AppError::new("recovery_destination_mismatch"));
            }
            intent
        } else {
            if MappingData::read(&source)
                .and_then(|data| data.validate_binding(pair.id, &pair.target_folder))
                .is_ok()
            {
                return Err(AppError::new("mapping_not_broken"));
            }
            let mut others = vec![config.clone(), pair.target_folder.clone()];
            for other in settings
                .sync_pairs
                .iter()
                .filter(|other| other.id != pair_id)
            {
                others.extend([other.source_folder.clone(), other.target_folder.clone()]);
            }
            let (_, destination) = validate_roots(&source, destination, &others)?;
            if fs::read_dir(&destination)?.next().is_some() {
                return Err(AppError::new("target_not_empty"));
            }
            let mut new_pair = pair.clone();
            new_pair.id = Uuid::new_v4();
            new_pair.processing_revision = Uuid::new_v4();
            new_pair.target_folder = destination;
            new_pair.processing_fingerprint = None;
            new_pair.created_at = OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| AppError::new("invalid_recovery"))?;
            let mut after = settings.clone();
            after.sync_pairs[index] = new_pair.clone();
            if after.selected_sync_pair_id == Some(pair_id) {
                after.selected_sync_pair_id = Some(new_pair.id);
            }
            let old_mapping = read_optional(&source, &source.join(MAPPING_FILE))?;
            let metadata = source.join("_redactio");
            let private_metadata_present = match fs::symlink_metadata(&metadata) {
                Ok(_) => {
                    reject_links(&metadata)?;
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            };
            let backup = source.join(format!(".redactio-backup-{}", new_pair.id));
            ValidatedWrite::new(&source, &backup)?.create_directory()?;
            if let Some(raw) = &old_mapping {
                ValidatedWrite::new(&source, &backup.join(MAPPING_FILE))?.write_atomic(raw)?;
            }
            let intent = Intent {
                schema_version: 1,
                old_pair: pair,
                new_mapping_hash: hash(&bytes(&MappingData::empty(
                    new_pair.id,
                    new_pair.target_folder.clone(),
                ))?),
                new_pair,
                settings_before_hash: hash(&bytes(&settings)?),
                settings_after_hash: hash(&bytes(&after)?),
                old_mapping_hash: old_mapping.as_deref().map(hash),
                private_metadata_present,
                stage: Stage::Prepared,
            };
            save_intent(&source, &intent)?;
            after_stage(Stage::Prepared)?;
            intent
        };
    let mut others = vec![config, intent.old_pair.target_folder.clone()];
    for other in settings
        .sync_pairs
        .iter()
        .filter(|other| other.id != pair_id)
    {
        others.extend([other.source_folder.clone(), other.target_folder.clone()]);
    }
    validate_roots(&source, &intent.new_pair.target_folder, &others)?;
    if fs::read_dir(&intent.new_pair.target_folder)?
        .next()
        .is_some()
    {
        return Err(AppError::new("target_not_empty"));
    }
    let settings_hash = hash(&bytes(&settings)?);
    if settings_hash != intent.settings_before_hash && settings_hash != intent.settings_after_hash {
        return Err(AppError::new("recovery_settings_changed"));
    }
    if intent.new_mapping_hash
        != hash(&bytes(&MappingData::empty(
            intent.new_pair.id,
            intent.new_pair.target_folder.clone(),
        ))?)
    {
        return Err(AppError::new("invalid_recovery"));
    }
    let mut after = settings.clone();
    after.sync_pairs[index] = intent.new_pair.clone();
    if after.selected_sync_pair_id == Some(intent.old_pair.id) {
        after.selected_sync_pair_id = Some(intent.new_pair.id);
    }
    if hash(&bytes(&after)?) != intent.settings_after_hash {
        return Err(AppError::new("invalid_recovery"));
    }
    let backup = source.join(format!(".redactio-backup-{}", intent.new_pair.id));
    reject_links(&backup)?;
    if hash_output(&source, &backup.join(MAPPING_FILE))? != intent.old_mapping_hash {
        return Err(AppError::new("recovery_backup_changed"));
    }
    let mapping_write = ValidatedWrite::new(&source, &source.join(MAPPING_FILE))?;
    let actual_hash = hash_output(&source, &source.join(MAPPING_FILE))?;
    if actual_hash != intent.old_mapping_hash
        && actual_hash.as_deref() != Some(&intent.new_mapping_hash)
    {
        return Err(AppError::new("recovery_mapping_changed"));
    }
    let live_metadata = source.join("_redactio");
    let saved_metadata = backup.join("_redactio");
    match (
        fs::symlink_metadata(&live_metadata),
        fs::symlink_metadata(&saved_metadata),
        intent.private_metadata_present,
    ) {
        (Ok(_), Err(error), true) if error.kind() == std::io::ErrorKind::NotFound => {
            guard.validate(&source)?;
            retain_private_metadata(&source, &backup)?;
        }
        (Err(error), Ok(_), true) if error.kind() == std::io::ErrorKind::NotFound => {
            reject_links(&saved_metadata)?;
        }
        (Err(a), Err(b), false)
            if a.kind() == std::io::ErrorKind::NotFound
                && b.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(AppError::new("recovery_metadata_changed")),
    }
    intent.stage = Stage::MetadataRetained;
    save_intent(&source, &intent)?;
    after_stage(Stage::MetadataRetained)?;
    if actual_hash.as_deref() != Some(&intent.new_mapping_hash) {
        guard.validate(&source)?;
        mapping_write.write_atomic(&bytes(&MappingData::empty(
            intent.new_pair.id,
            intent.new_pair.target_folder.clone(),
        ))?)?;
    }
    intent.stage = Stage::MappingPublished;
    save_intent(&source, &intent)?;
    after_stage(Stage::MappingPublished)?;
    if settings_hash != intent.settings_after_hash {
        settings = after;
        save_settings(settings_path, &settings)?;
    }
    after_stage(Stage::Complete)?;
    intent.stage = Stage::Complete;
    save_intent(&source, &intent)?;
    ensure_recovered(&source, intent.new_pair.id)?;
    MappingData::read(&source)?
        .validate_binding(intent.new_pair.id, &intent.new_pair.target_folder)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interrupted_stage_reloads_and_resumes_with_one_new_identity() {
        for interrupted in [
            Stage::Prepared,
            Stage::MetadataRetained,
            Stage::MappingPublished,
            Stage::Complete,
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let target = root.path().join("target");
            let destination = root.path().join("fresh");
            let config = root.path().join("config");
            for path in [&source, &target, &destination, &config] {
                fs::create_dir(path).unwrap();
            }
            let path = config.join("settings.json");
            let mut settings = Settings::default();
            let old_id = settings.add("pair", &source, &target).unwrap();
            save_settings(&path, &settings).unwrap();
            fs::write(source.join(MAPPING_FILE), b"broken").unwrap();
            fs::create_dir_all(source.join("_redactio/reviews")).unwrap();
            fs::write(source.join("_redactio/reviews/doc-0001.json"), b"review").unwrap();
            let error = fresh_start_with(&path, old_id, &destination, true, |stage| {
                if stage == interrupted {
                    Err(AppError::new("injected_stop"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(error.code, "injected_stop");
            let intent = read_intent(&source).unwrap().unwrap();
            let expected_id = intent.new_pair.id;
            let saved = load_settings(&path).unwrap();
            assert_eq!(
                saved
                    .validate_registry(std::slice::from_ref(&config))
                    .unwrap_err()
                    .code,
                "mapping_recovery_required"
            );
            let candidate = recovery_pairs(&path).unwrap().remove(0);
            assert!(
                same_directory(candidate.pending_target.as_deref().unwrap(), &destination).unwrap()
            );
            let resumed = fresh_start(&path, candidate.id, &destination, true).unwrap();
            assert_eq!(resumed.sync_pairs[0].id, expected_id);
            assert_eq!(
                fs::read(source.join(format!(
                    ".redactio-backup-{expected_id}/_redactio/reviews/doc-0001.json"
                )))
                .unwrap(),
                b"review"
            );
            assert!(recovery_pairs(&path).unwrap().is_empty());
        }
    }

    #[test]
    fn retry_refuses_tampered_backup_and_never_replaces_new_disk_truth() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let path = config.path().join("settings.json");
        let mut settings = Settings::default();
        let id = settings.add("pair", source.path(), target.path()).unwrap();
        save_settings(&path, &settings).unwrap();
        fs::write(source.path().join(MAPPING_FILE), b"broken").unwrap();
        fresh_start_with(&path, id, destination.path(), true, |_| {
            Err(AppError::new("injected_stop"))
        })
        .unwrap_err();
        let intent = read_intent(source.path()).unwrap().unwrap();
        let backup = source.path().join(format!(
            ".redactio-backup-{}/{}",
            intent.new_pair.id, MAPPING_FILE
        ));
        fs::write(&backup, b"changed backup").unwrap();
        assert_eq!(
            fresh_start(&path, id, destination.path(), true)
                .unwrap_err()
                .code,
            "recovery_backup_changed"
        );
        assert_eq!(
            fs::read(source.path().join(MAPPING_FILE)).unwrap(),
            b"broken"
        );
        assert_eq!(load_settings(&path).unwrap(), settings);
    }
}
