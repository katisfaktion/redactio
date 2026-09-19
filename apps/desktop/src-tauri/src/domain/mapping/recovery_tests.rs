use super::*;
use crate::{
    domain::settings::{save_settings, Settings, SyncPair},
    protocol::{Decisions, DocumentKey, EngineInfo, ReviewStatus},
};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, fs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommitStage {
    Reserved,
    Journaled,
    OutputStaged,
    ReviewStaged,
    OutputReplaced,
    ReviewReplaced,
}
type Hook = Box<dyn FnMut(CommitStage) -> Result<(), AppError>>;
thread_local! { static HOOK: RefCell<Option<Hook>> = RefCell::new(None); }
pub(super) fn checkpoint(stage: CommitStage) -> Result<(), AppError> {
    HOOK.with_borrow_mut(|hook| hook.as_mut().map_or(Ok(()), |hook| hook(stage)))
}
fn stop_at(stage: CommitStage) {
    HOOK.with_borrow_mut(|hook| {
        *hook = Some(Box::new(move |actual| {
            if stage == actual {
                Err(AppError::new("test_interruption"))
            } else {
                Ok(())
            }
        }))
    });
}
fn clear_hook() {
    HOOK.with_borrow_mut(|hook| *hook = None);
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
struct Fixture {
    pair: SyncPair,
    settings: PathBuf,
    config_guard: Option<CollectionGuard>,
    guard: Option<CollectionGuard>,
    mapping: Mapping,
    candidate: CommitCandidate,
    // Drop the guards before TempDir, including on Windows with delete-sharing denied.
    _root: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        let config = root.path().join("config");
        for path in [&source, &target, &config] {
            fs::create_dir(path).unwrap();
        }
        fs::write(source.join("a.docx"), b"synthetic source").unwrap();
        let mut settings = Settings::default();
        let id = settings.add("synthetic", &source, &target).unwrap();
        let settings_path = config.join("settings.json");
        save_settings(&settings_path, &settings).unwrap();
        let pair = settings.sync_pairs.remove(0);
        let config_guard = CollectionGuard::acquire(&config).unwrap();
        let guard = CollectionGuard::acquire(&source).unwrap();
        let mut mapping = Mapping::load(&source, id, &target).unwrap();
        let doc_id = mapping.reserve_with_guard("a.docx", &guard).unwrap();
        let key = DocumentKey {
            sync_pair_id: id,
            doc_id,
        };
        let review = ReviewRecord {
            schema_version: 1,
            key: key.clone(),
            source_hash: hash(b"synthetic source"),
            revision: pair.processing_revision,
            detections: vec![],
            decisions: Decisions::default(),
            status: ReviewStatus::Pending,
            notes: "private".into(),
            acknowledged_warnings: vec![],
            warnings: vec![],
            redacted_at: "2026-09-19T12:00:00Z".into(),
            reviewed_at: None,
            engine: EngineInfo {
                engine_version: "1".into(),
                model_name: "synthetic".into(),
                model_version: "1".into(),
                recognizers: vec![],
                extraction_version: "1".into(),
            },
            output_hash: hash(b"pending markdown"),
        };
        let candidate = CommitCandidate {
            key,
            source_hash: review.source_hash.clone(),
            revision: pair.processing_revision,
            markdown: b"pending markdown".to_vec(),
            review,
        };
        Self {
            _root: root,
            pair,
            settings: settings_path,
            config_guard: Some(config_guard),
            guard: Some(guard),
            mapping,
            candidate,
        }
    }
    fn commit(&mut self, expected: Option<&str>) -> Result<Generation, AppError> {
        commit_generation(
            &self.pair,
            &mut self.mapping,
            self.candidate.clone(),
            expected,
            self.guard.as_ref().unwrap(),
            &self.settings,
            self.config_guard.as_ref().unwrap(),
        )
    }
    fn recover(&mut self) -> Result<(), AppError> {
        // Simulate process restart: discard in-memory mapping and reacquire OS locks in order.
        let source = self.pair.source_folder.clone();
        let target = self.pair.target_folder.clone();
        drop(self.guard.take());
        drop(self.config_guard.take());
        self.config_guard = Some(CollectionGuard::acquire(self.settings.parent().unwrap())?);
        self.guard = Some(CollectionGuard::acquire(&source)?);
        self.mapping = Mapping::load(&source, self.pair.id, &target)?;
        recover_pending(
            &self.pair,
            &mut self.mapping,
            self.guard.as_ref().unwrap(),
            &self.settings,
            self.config_guard.as_ref().unwrap(),
        )
    }
    fn output(&self) -> PathBuf {
        self.pair.target_folder.join("doc-0001.md")
    }
    fn review(&self) -> PathBuf {
        self.pair
            .source_folder
            .join("_redactio/reviews/doc-0001.json")
    }
    fn approve(&mut self) {
        self.candidate.markdown = b"approved markdown".to_vec();
        self.candidate.review.output_hash = hash(&self.candidate.markdown);
        self.candidate.review.status = ReviewStatus::Approved;
        self.candidate.review.reviewed_at = Some("2026-09-19T13:00:00Z".into());
    }
    fn assert_committed(&mut self) {
        let entry = &self.mapping.entries()[0];
        assert!(entry.pending.is_none());
        let generation = entry.committed.as_ref().unwrap();
        assert_eq!(
            generation.output_hash,
            hash(&fs::read(self.output()).unwrap())
        );
        assert_eq!(
            generation.review_hash,
            hash(&fs::read(self.review()).unwrap())
        );
        assert_eq!(fs::read(self.output()).unwrap(), self.candidate.markdown);
        assert_eq!(self.mapping.entries().len(), 1);
        assert_eq!(
            self.mapping
                .reserve_with_guard("a.docx", self.guard.as_ref().unwrap())
                .unwrap(),
            "doc-0001"
        );
        assert_eq!(
            self.mapping
                .reserve_with_guard("b.docx", self.guard.as_ref().unwrap())
                .unwrap(),
            "doc-0002"
        );
    }
}
fn exercise_commit_crash(stage: CommitStage) -> Result<(), AppError> {
    let mut f = Fixture::new();
    stop_at(stage);
    let result = f.commit(None);
    clear_hook();
    assert_eq!(result.unwrap_err().code, "test_interruption", "{stage:?}");
    f.recover()?;
    if f.mapping.entries()[0].committed.is_none() {
        f.commit(None)?;
    }
    f.assert_committed();
    Ok(())
}
#[test]
fn every_commit_boundary_is_recoverable_without_id_reuse() {
    for stage in [
        CommitStage::Reserved,
        CommitStage::Journaled,
        CommitStage::OutputStaged,
        CommitStage::ReviewStaged,
        CommitStage::OutputReplaced,
        CommitStage::ReviewReplaced,
    ] {
        exercise_commit_crash(stage).unwrap();
    }
}
#[test]
fn approved_save_crashes_never_publish_current_without_matching_private_metadata() {
    for stage in [
        CommitStage::Reserved,
        CommitStage::Journaled,
        CommitStage::OutputStaged,
        CommitStage::ReviewStaged,
        CommitStage::OutputReplaced,
        CommitStage::ReviewReplaced,
    ] {
        let mut f = Fixture::new();
        let old = f.commit(None).unwrap();
        f.approve();
        stop_at(stage);
        let result = f.commit(Some(&old.output_hash));
        clear_hook();
        assert_eq!(result.unwrap_err().code, "test_interruption");
        if stage != CommitStage::Reserved {
            assert_eq!(
                crate::domain::scan::scan_collection(&f.pair).unwrap().files[0].state,
                DocumentState::RecoveryPending
            );
        }
        f.recover().unwrap();
        if f.mapping.entries()[0]
            .committed
            .as_ref()
            .unwrap()
            .output_hash
            != f.candidate.review.output_hash
        {
            f.commit(Some(&old.output_hash)).unwrap();
        }
        assert_eq!(
            f.mapping.entries()[0]
                .committed
                .as_ref()
                .unwrap()
                .first_processed_at,
            old.first_processed_at
        );
        f.assert_committed();
        assert_eq!(
            ReviewRecord::read(
                &f.pair.source_folder,
                &f.candidate.key,
                f.mapping.entries()[0].committed.as_ref().unwrap()
            )
            .unwrap()
            .status,
            ReviewStatus::Approved
        );
    }
}

#[test]
fn automatic_extraction_warnings_allow_needs_rework_without_a_human_timestamp() {
    let mut f = Fixture::new();
    f.candidate.review.status = ReviewStatus::NeedsRework;
    f.candidate.review.warnings = vec!["images".into()];
    f.commit(None).unwrap();
    assert_eq!(
        ReviewRecord::read(
            &f.pair.source_folder,
            &f.candidate.key,
            f.mapping.entries()[0].committed.as_ref().unwrap()
        )
        .unwrap()
        .status,
        ReviewStatus::NeedsRework
    );
}
#[test]
fn empty_extraction_cannot_be_approved_even_when_acknowledged() {
    let mut f = Fixture::new();
    f.approve();
    f.candidate.review.warnings = vec!["empty_document".into()];
    f.candidate.review.acknowledged_warnings = vec!["empty_document".into()];
    assert_eq!(f.commit(None).unwrap_err().code, "invalid_review");
    assert!(!f.output().exists());
}
#[test]
fn source_or_settings_changes_during_staging_do_not_publish() {
    for config_change in [false, true] {
        let mut f = Fixture::new();
        let source = f.pair.source_folder.join("a.docx");
        let settings = f.settings.clone();
        HOOK.with_borrow_mut(|hook| {
            *hook = Some(Box::new(move |stage| {
                if stage == CommitStage::ReviewStaged {
                    if config_change {
                        let mut saved = crate::domain::settings::load_settings(&settings)?;
                        saved.sync_pairs[0].processing_revision = Uuid::new_v4();
                        save_settings(&settings, &saved)?;
                    } else {
                        fs::write(&source, b"changed source")?;
                    }
                }
                Ok(())
            }))
        });
        let result = f.commit(None);
        clear_hook();
        assert_eq!(
            result.unwrap_err().code,
            if config_change {
                "revision_changed"
            } else {
                "source_changed"
            }
        );
        assert!(!f.output().exists());
        f.recover().unwrap();
        let current = crate::domain::settings::load_settings(&f.settings)
            .unwrap()
            .sync_pairs
            .remove(0);
        assert_eq!(
            crate::domain::scan::scan_collection(&current)
                .unwrap()
                .files[0]
                .state,
            DocumentState::Stale
        );
    }
}
#[test]
fn alien_outputs_and_missing_or_edited_review_outputs_are_never_overwritten() {
    for kind in ["alien", "edited", "missing", "recreated"] {
        let mut f = Fixture::new();
        let old = if kind == "alien" {
            None
        } else {
            Some(f.commit(None).unwrap())
        };
        if kind != "alien" {
            f.approve();
        }
        if kind == "missing" {
            fs::remove_file(f.output()).unwrap();
        } else {
            fs::write(f.output(), kind.as_bytes()).unwrap();
        }
        let before = fs::read(f.output()).ok();
        assert!(f
            .commit(old.as_ref().map(|g| g.output_hash.as_str()))
            .is_err());
        assert_eq!(fs::read(f.output()).ok(), before);
    }
}
#[test]
fn later_creator_after_missing_output_observation_is_a_conflict() {
    let mut f = Fixture::new();
    f.commit(None).unwrap();
    fs::remove_file(f.output()).unwrap();
    let output = f.output();
    HOOK.with_borrow_mut(|hook| {
        *hook = Some(Box::new(move |stage| {
            if stage == CommitStage::ReviewStaged {
                fs::write(&output, b"later owner")?;
            }
            Ok(())
        }))
    });
    let result = f.commit(None);
    clear_hook();
    assert_eq!(result.unwrap_err().code, "output_conflict");
    assert_eq!(f.recover().unwrap_err().code, "output_conflict");
    assert_eq!(fs::read(f.output()).unwrap(), b"later owner");
}
#[test]
fn partial_publish_with_lost_stage_remains_pending_until_exact_retry() {
    let mut f = Fixture::new();
    let old = f.commit(None).unwrap();
    f.approve();
    stop_at(CommitStage::OutputReplaced);
    let result = f.commit(Some(&old.output_hash));
    clear_hook();
    assert!(result.is_err());
    let pending = f.mapping.entries()[0].pending.as_ref().unwrap();
    fs::remove_file(f.review().parent().unwrap().join(&pending.review_temporary)).unwrap();
    f.recover().unwrap();
    assert!(f.mapping.entries()[0].pending.is_some());
    assert_eq!(
        crate::domain::scan::scan_collection(&f.pair).unwrap().files[0].state,
        DocumentState::RecoveryPending
    );
    f.commit(Some(&old.output_hash)).unwrap();
    f.assert_committed();
}
#[test]
fn edited_review_and_temporary_files_fail_closed_and_remain_intact() {
    for temporary in [false, true] {
        let mut f = Fixture::new();
        let old = f.commit(None).unwrap();
        f.approve();
        stop_at(CommitStage::ReviewStaged);
        let result = f.commit(Some(&old.output_hash));
        clear_hook();
        assert!(result.is_err());
        let path = if temporary {
            f.pair.target_folder.join(
                &f.mapping.entries()[0]
                    .pending
                    .as_ref()
                    .unwrap()
                    .output_temporary,
            )
        } else {
            f.review()
        };
        fs::write(&path, b"external").unwrap();
        assert_eq!(f.recover().unwrap_err().code, "output_conflict");
        assert_eq!(fs::read(&path).unwrap(), b"external");
        assert!(f.mapping.entries()[0].pending.is_some());
        assert_eq!(fs::read(f.output()).unwrap(), b"pending markdown");
    }
}
#[cfg(unix)]
#[test]
fn real_disk_write_failure_preserves_pending_then_recovers() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    let target = f.pair.target_folder.clone();
    HOOK.with_borrow_mut(|hook| {
        *hook = Some(Box::new(move |stage| {
            if stage == CommitStage::Journaled {
                fs::set_permissions(&target, fs::Permissions::from_mode(0o500))?;
            }
            Ok(())
        }))
    });
    let result = f.commit(None);
    clear_hook();
    fs::set_permissions(&f.pair.target_folder, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result.unwrap_err().code, "permission_denied");
    f.recover().unwrap();
    f.commit(None).unwrap();
    f.assert_committed();
}
#[cfg(windows)]
#[test]
fn native_locked_output_preserves_prior_bytes_and_recovers_after_unlock() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut f = Fixture::new();
    let old = f.commit(None).unwrap();
    f.approve();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.output())
        .unwrap();
    let error = f.commit(Some(&old.output_hash)).unwrap_err();
    assert_eq!(error.code, "file_busy");
    assert!(error.retryable);
    drop(lock);
    f.recover().unwrap();
    if f.mapping.entries()[0]
        .committed
        .as_ref()
        .unwrap()
        .output_hash
        != f.candidate.review.output_hash
    {
        f.commit(Some(&old.output_hash)).unwrap();
    }
    f.assert_committed();
}

#[test]
fn indeterminate_storage_errors_reconcile_actual_published_bytes() {
    use crate::domain::storage::tests::FAIL_AFTER_WRITE;
    for code in [
        "storage_cleanup_required",
        "storage_durability_uncertain",
        "storage_recovery_required",
    ] {
        for destination in ["output", "review", "mapping"] {
            let mut f = Fixture::new();
            let old = f.commit(None).unwrap();
            f.approve();
            let path = match destination {
                "output" => f.output(),
                "review" => f.review(),
                _ => f.pair.source_folder.join(MAPPING_FILE),
            };
            HOOK.with_borrow_mut(|hook| {
                *hook = Some(Box::new(move |stage| {
                    if stage == CommitStage::ReviewStaged {
                        FAIL_AFTER_WRITE.with_borrow_mut(|failure| {
                            *failure = Some((path.clone(), code, false))
                        });
                    }
                    Ok(())
                }))
            });
            let result = f.commit(Some(&old.output_hash));
            clear_hook();
            FAIL_AFTER_WRITE.with_borrow_mut(|failure| *failure = None);
            assert_eq!(result.unwrap_err().code, code, "{destination}");
            f.recover().unwrap();
            f.assert_committed();
        }
    }
}
#[test]
fn missing_owned_output_regenerates_using_the_existing_id() {
    let mut f = Fixture::new();
    let old = f.commit(None).unwrap();
    fs::remove_file(f.output()).unwrap();
    let new = f.commit(None).unwrap();
    assert_eq!(new.first_processed_at, old.first_processed_at);
    f.assert_committed();
}
#[test]
fn persisted_journal_rejects_paths_wrong_owners_and_inconsistent_hashes() {
    for field in [
        "output_temporary",
        "review_temporary",
        "owner",
        "review_hash",
        "unknown",
    ] {
        let mut f = Fixture::new();
        stop_at(CommitStage::Journaled);
        let result = f.commit(None);
        clear_hook();
        assert!(result.is_err());
        let path = f.pair.source_folder.join(MAPPING_FILE);
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let pending = &mut data["entries"][0]["pending"];
        match field {
            "output_temporary" | "review_temporary" => pending[field] = "../../outside".into(),
            "owner" => pending["review"]["key"]["sync_pair_id"] = Uuid::new_v4().to_string().into(),
            "review_hash" => pending["generation"]["review_hash"] = "a".repeat(64).into(),
            _ => pending["original_text"] = "must be rejected".into(),
        }
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(f.recover().is_err());
        assert!(!f.output().exists());
    }
}
#[test]
fn invalid_private_review_and_wrong_pair_or_lock_cannot_commit() {
    for field in [
        "owner",
        "doc_id",
        "hash",
        "timestamp",
        "span",
        "guard",
        "config_guard",
    ] {
        let mut f = Fixture::new();
        match field {
            "owner" => f.candidate.key.sync_pair_id = Uuid::new_v4(),
            "doc_id" => {
                f.candidate.key.doc_id = "../outside".into();
                f.candidate.review.key.doc_id = "../outside".into();
            }
            "hash" => f.candidate.review.output_hash = "a".repeat(64),
            "timestamp" => f.candidate.review.redacted_at = "2026-09-19T12:00:00".into(),
            "span" => f
                .candidate
                .review
                .decisions
                .dismissed_ids
                .push("not-present".into()),
            "guard" => {
                let other = f._root.path().join("other");
                fs::create_dir(&other).unwrap();
                f.guard = Some(CollectionGuard::acquire(&other).unwrap());
            }
            "config_guard" => {
                let other = f._root.path().join("other");
                fs::create_dir(&other).unwrap();
                f.config_guard = Some(CollectionGuard::acquire(&other).unwrap());
            }
            _ => unreachable!(),
        }
        assert!(f.commit(None).is_err(), "{field}");
        assert!(!f.output().exists());
    }
}
#[cfg(unix)]
#[test]
fn redirected_private_metadata_is_rejected_without_touching_outside_data() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new();
    let outside = f._root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("canary"), b"outside").unwrap();
    symlink(&outside, f.pair.source_folder.join("_redactio")).unwrap();
    assert!(f.commit(None).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    assert_eq!(fs::read(outside.join("canary")).unwrap(), b"outside");
}

#[test]
fn externally_changed_mapping_is_rejected_before_output_publication() {
    let mut f = Fixture::new();
    let old = f.commit(None).unwrap();
    f.approve();
    let path = f.pair.source_folder.join(MAPPING_FILE);
    HOOK.with_borrow_mut(|hook| {
        *hook = Some(Box::new(move |stage| {
            if stage == CommitStage::ReviewStaged {
                let mut disk: MappingData = serde_json::from_slice(&fs::read(&path)?).unwrap();
                disk.entries[0].pending = None;
                fs::write(&path, serde_json::to_vec_pretty(&disk).unwrap())?;
            }
            Ok(())
        }))
    });
    let result = f.commit(Some(&old.output_hash));
    clear_hook();
    assert_eq!(result.unwrap_err().code, "mapping_changed");
    assert_eq!(fs::read(f.output()).unwrap(), b"pending markdown");
}
#[test]
fn journal_timestamp_must_match_the_hashed_review_metadata() {
    let mut f = Fixture::new();
    stop_at(CommitStage::Journaled);
    let result = f.commit(None);
    clear_hook();
    assert!(result.is_err());
    let path = f.pair.source_folder.join(MAPPING_FILE);
    let mut data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    data["entries"][0]["pending"]["generation"]["last_processed_at"] =
        "2026-09-20T12:00:00Z".into();
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(f.recover().is_err());
}

#[test]
fn private_record_wire_validation_reuses_canonical_ids_and_safe_codes() {
    let f = Fixture::new();
    for field in ["revision", "warnings"] {
        let mut data = serde_json::to_value(&f.candidate.review).unwrap();
        if field == "revision" {
            data[field] = f.pair.processing_revision.simple().to_string().into();
        } else {
            data[field] = serde_json::json!(["_invalid"]);
        }
        let record = serde_json::from_value::<ReviewRecord>(data);
        assert!(
            record.is_err() || record.unwrap().validate().is_err(),
            "{field}"
        );
    }
}

#[test]
fn opening_private_review_checks_the_committed_bytes_and_identity() {
    let mut f = Fixture::new();
    let generation = f.commit(None).unwrap();
    let record = ReviewRecord::read(&f.pair.source_folder, &f.candidate.key, &generation).unwrap();
    assert_eq!(record.notes, "private");
    let mut edited = record;
    edited.notes = "external note".into();
    fs::write(f.review(), serde_json::to_vec_pretty(&edited).unwrap()).unwrap();
    assert_eq!(
        ReviewRecord::read(&f.pair.source_folder, &f.candidate.key, &generation)
            .unwrap_err()
            .code,
        "review_mismatch"
    );
}

#[test]
fn missing_mapping_after_partial_native_replacement_preserves_error_and_backup() {
    use crate::domain::storage::tests::FAIL_AFTER_WRITE;
    let mut f = Fixture::new();
    let old = f.commit(None).unwrap();
    f.approve();
    let path = f.pair.source_folder.join(MAPPING_FILE);
    let injected = path.clone();
    HOOK.with_borrow_mut(|hook| {
        *hook = Some(Box::new(move |stage| {
            if stage == CommitStage::ReviewStaged {
                FAIL_AFTER_WRITE.with_borrow_mut(|failure| {
                    *failure = Some((injected.clone(), "storage_recovery_required", true))
                });
            }
            Ok(())
        }))
    });
    let result = f.commit(Some(&old.output_hash));
    clear_hook();
    FAIL_AFTER_WRITE.with_borrow_mut(|failure| *failure = None);
    assert_eq!(result.unwrap_err().code, "storage_recovery_required");
    assert!(path.with_extension("retained").exists());
    assert!(!path.exists());
    assert!(f.recover().is_err());
    fs::rename(path.with_extension("retained"), &path).unwrap();
    f.recover().unwrap();
    f.assert_committed();
}
