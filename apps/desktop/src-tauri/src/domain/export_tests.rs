use crate::{
    domain::{
        detection,
        export::export_approved,
        mapping::{commit_generation, CollectionGuard, CommitCandidate, Mapping, ReviewRecord},
        settings::{load_settings, save_settings, Settings},
        sync::RunController,
    },
    protocol::{Decisions, DocumentKey, EngineInfo, ReviewStatus},
    sidecar::Sidecar,
};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

struct Fixture {
    root: tempfile::TempDir,
    path: PathBuf,
    controller: RunController,
    key: DocumentKey,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        let mut settings = Settings::default();
        for name in ["A", "B"] {
            let source = root.path().join(format!("{name}-source"));
            let target = root.path().join(format!("{name}-target"));
            fs::create_dir(&source).unwrap();
            fs::create_dir(&target).unwrap();
            settings.add(name, &source, &target).unwrap();
        }
        let key = DocumentKey {
            sync_pair_id: settings.sync_pairs[0].id,
            doc_id: "doc-0001".into(),
        };
        let path = config.join("settings.json");
        save_settings(&path, &settings).unwrap();
        Self {
            controller: RunController::new(path.clone()),
            root,
            path,
            key,
        }
    }
    fn sidecar(&self, mode: &str) -> Sidecar {
        Sidecar::new(
            std::env::var_os("REDACTIO_TEST_PYTHON").unwrap().into(),
            vec![
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fake_sidecar.py")
                    .into_os_string(),
                mode.into(),
                self.root.path().join("engine-version").into_os_string(),
            ],
            self.root.path().into(),
        )
    }
    async fn seed(&self, sidecar: &Sidecar, status: ReviewStatus, filename: &str) {
        let settings =
            detection::refresh_processing_config(&self.controller, sidecar, self.key.sync_pair_id)
                .await
                .unwrap();
        let pair = settings
            .sync_pairs
            .iter()
            .find(|pair| pair.id == self.key.sync_pair_id)
            .unwrap();
        fs::write(pair.source_folder.join(filename), b"synthetic").unwrap();
        let config_guard = CollectionGuard::acquire(self.path.parent().unwrap()).unwrap();
        let source_guard = CollectionGuard::acquire(&pair.source_folder).unwrap();
        let mut mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
        let key = DocumentKey {
            sync_pair_id: pair.id,
            doc_id: mapping.reserve_with_guard(filename, &source_guard).unwrap(),
        };
        let markdown = b"previously approved synthetic bytes".to_vec();
        let output_hash = format!("{:x}", Sha256::digest(&markdown));
        let source_hash = format!("{:x}", Sha256::digest(b"synthetic"));
        let record = ReviewRecord {
            schema_version: 1,
            key: key.clone(),
            source_hash: source_hash.clone(),
            revision: pair.processing_revision,
            detections: vec![],
            decisions: Decisions::default(),
            status,
            notes: "PRIVATE_CANARY".into(),
            acknowledged_warnings: vec![],
            warnings: vec![],
            redacted_at: "2026-01-01T00:00:00Z".into(),
            reviewed_at: matches!(status, ReviewStatus::Approved | ReviewStatus::Rejected)
                .then(|| "2026-01-02T00:00:00Z".into()),
            engine: EngineInfo {
                engine_version: "synthetic-engine".into(),
                model_name: pair.config.model.clone(),
                model_version: "synthetic-model".into(),
                extraction_version: "synthetic-extraction".into(),
                recognizers: vec!["synthetic-recognizer".into()],
            },
            output_hash: output_hash.clone(),
        };
        commit_generation(
            pair,
            &mut mapping,
            CommitCandidate {
                key: key.clone(),
                source_hash,
                revision: pair.processing_revision,
                markdown,
                review: record,
            },
            None,
            &source_guard,
            &self.path,
            &config_guard,
        )
        .unwrap();
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn only_current_approved_bytes_are_exported_without_private_metadata() {
    runtime().block_on(async {
        for status in [
            ReviewStatus::Pending,
            ReviewStatus::Rejected,
            ReviewStatus::NeedsRework,
            ReviewStatus::Approved,
        ] {
            let f = Fixture::new();
            let sidecar = f.sidecar("detection");
            f.seed(&sidecar, status, "synthetic.docx").await;
            let destination = f.root.path().join("export");
            fs::create_dir(&destination).unwrap();
            let pair = load_settings(&f.path).unwrap().sync_pairs.remove(0);
            let before =
                fs::read(pair.source_folder.join("_redactio/reviews/doc-0001.json")).unwrap();
            let result = export_approved(
                &f.controller,
                pair.id,
                vec!["doc-0001".into()],
                Some(&destination),
                || Ok(sidecar.clone()),
            )
            .await
            .unwrap();
            assert_eq!(result.sync_pair_id, pair.id);
            assert!(!result.audit_warning);
            assert!(!result.cancelled);
            assert!(result.error.is_none());
            if status == ReviewStatus::Approved {
                assert_eq!(result.exported, ["doc-0001"]);
                assert!(result.failed.is_empty());
                assert_eq!(
                    fs::read(destination.join("doc-0001.md")).unwrap(),
                    b"previously approved synthetic bytes"
                );
                assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
            } else {
                assert!(result.exported.is_empty());
                assert_eq!(result.failed[0].doc_id, "doc-0001");
                assert_eq!(result.failed[0].error.code, "approval_required");
                assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            }
            assert_eq!(
                fs::read(pair.source_folder.join("_redactio/reviews/doc-0001.json")).unwrap(),
                before
            );
            let audit =
                fs::read_to_string(f.path.parent().unwrap().join("audit-log.jsonl")).unwrap();
            assert!(audit.contains("\"action\":\"export\""));
            assert!(!audit.contains("PRIVATE_CANARY"));
            assert!(!audit.contains("synthetic.docx"));
            sidecar.shutdown().await;
        }
    });
}

#[test]
fn invalid_or_mixed_selection_is_rejected_before_creating_any_files() {
    runtime().block_on(async {
        let mut f = Fixture::new();
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "synthetic.docx")
            .await;
        let selected = f.key.clone();
        f.key.sync_pair_id = load_settings(&f.path).unwrap().sync_pairs[1].id;
        f.seed(&sidecar, ReviewStatus::Approved, "foreign-first.docx")
            .await;
        f.seed(&sidecar, ReviewStatus::Approved, "foreign-second.docx")
            .await;
        f.key = selected;
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        for (ids, code) in [
            (vec![], "invalid_export_selection"),
            (vec!["doc-0001", "doc-0001"], "invalid_export_selection"),
            (vec!["doc-0001", "../private"], "invalid_export_selection"),
            (vec!["doc-0001", "doc-0002"], "unknown_document"),
        ] {
            let result = export_approved(
                &f.controller,
                f.key.sync_pair_id,
                ids.into_iter().map(str::to_owned).collect(),
                Some(&destination),
                || Ok(sidecar.clone()),
            )
            .await
            .unwrap();
            assert_eq!(result.error.unwrap().code, code);
            assert!(result.exported.is_empty());
            assert!(result.failed.is_empty());
            assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        }
        sidecar.shutdown().await;
    });
}

#[test]
fn stale_tampered_empty_and_foreign_records_cannot_export() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "synthetic.docx")
            .await;
        let pair = load_settings(&f.path).unwrap().sync_pairs.remove(0);
        let source = pair.source_folder.join("synthetic.docx");
        let output = pair.target_folder.join("doc-0001.md");
        let review = pair.source_folder.join("_redactio/reviews/doc-0001.json");
        let mapping = pair.source_folder.join("_document-mapping.json");
        let paths = [&source, &output, &review, &mapping];
        let saved: Vec<_> = paths.iter().map(|path| fs::read(path).unwrap()).collect();
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        for mutation in [
            "source",
            "output",
            "review",
            "review-hash",
            "foreign-key",
            "empty",
            "output-binding",
            "engine",
            "revision",
        ] {
            match mutation {
                "source" => fs::write(&source, b"changed source").unwrap(),
                "output" => fs::write(&output, b"changed output").unwrap(),
                "review" => fs::write(&review, b"changed private record").unwrap(),
                "revision" => {
                    let mut settings = load_settings(&f.path).unwrap();
                    settings.sync_pairs[0].processing_revision = uuid::Uuid::new_v4();
                    save_settings(&f.path, &settings).unwrap();
                }
                _ => {
                    let mut record: serde_json::Value = serde_json::from_slice(&saved[2]).unwrap();
                    match mutation {
                        "foreign-key" => {
                            record["key"]["sync_pair_id"] = uuid::Uuid::new_v4().to_string().into()
                        }
                        "empty" => {
                            record["warnings"] = serde_json::json!(["empty_document"]);
                            record["acknowledged_warnings"] = serde_json::json!(["empty_document"]);
                        }
                        "output-binding" => record["output_hash"] = "a".repeat(64).into(),
                        "engine" => record["engine"]["engine_version"] = "foreign-engine".into(),
                        _ => (),
                    }
                    let bytes = serde_json::to_vec(&record).unwrap();
                    fs::write(&review, &bytes).unwrap();
                    let mut data: serde_json::Value = serde_json::from_slice(&saved[3]).unwrap();
                    data["entries"][0]["committed"]["review_hash"] = if mutation == "review-hash" {
                        "0".repeat(64)
                    } else {
                        format!("{:x}", Sha256::digest(&bytes))
                    }
                    .into();
                    fs::write(&mapping, serde_json::to_vec(&data).unwrap()).unwrap();
                }
            }
            let result = export_approved(
                &f.controller,
                pair.id,
                vec!["doc-0001".into()],
                Some(&destination),
                || Ok(sidecar.clone()),
            )
            .await
            .unwrap();
            assert!(result.error.is_none(), "{mutation}: {:?}", result.error);
            assert!(result.exported.is_empty(), "{mutation}");
            assert_eq!(result.failed.len(), 1, "{mutation}");
            let expected = match mutation {
                "source" | "revision" | "engine" => "reprocess_required",
                "output" => "output_conflict",
                "empty" => "invalid_review",
                _ => "review_mismatch",
            };
            assert_eq!(result.failed[0].error.code, expected, "{mutation}");
            assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            for (path, bytes) in paths.iter().zip(&saved) {
                fs::write(path, bytes).unwrap();
            }
        }
        sidecar.shutdown().await;
    });
}

#[test]
fn engine_refresh_invalidates_approval_and_partial_results_retain_successes() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "synthetic.docx")
            .await;
        f.seed(&sidecar, ReviewStatus::Pending, "second.docx").await;
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into(), "doc-0002".into()],
            Some(&destination),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        assert_eq!(result.exported, ["doc-0001"]);
        assert_eq!(result.failed[0].doc_id, "doc-0002");
        assert_eq!(result.failed[0].error.code, "approval_required");
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
        let before = load_settings(&f.path).unwrap();
        fs::write(f.root.path().join("engine-version"), "upgraded-engine").unwrap();
        let other = f.root.path().join("after-upgrade");
        fs::create_dir(&other).unwrap();
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into()],
            Some(&other),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        assert_eq!(result.failed[0].error.code, "reprocess_required");
        assert_eq!(fs::read_dir(&other).unwrap().count(), 0);
        let after = load_settings(&f.path).unwrap();
        assert_ne!(
            before.sync_pairs[0].processing_revision,
            after.sync_pairs[0].processing_revision
        );
        assert_eq!(before.sync_pairs[1], after.sync_pairs[1]);
        assert_eq!(
            fs::read(destination.join("doc-0001.md")).unwrap(),
            b"previously approved synthetic bytes"
        );
        assert!(f.controller.try_operation().is_ok());
        assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_ok());
        assert!(CollectionGuard::acquire(&after.sync_pairs[0].source_folder).is_ok());
        sidecar.shutdown().await;
    });
}

#[test]
fn cancellation_is_audited_without_resolving_sidecar_and_audit_failure_keeps_exports() {
    runtime().block_on(async {
        let f = Fixture::new();
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into()],
            None,
            || panic!("cancel must not resolve sidecar"),
        )
        .await
        .unwrap();
        assert!(result.cancelled);
        assert!(!result.audit_warning);
        assert!(result.exported.is_empty());
        let audit = f.path.parent().unwrap().join("audit-log.jsonl");
        let entry: serde_json::Value =
            serde_json::from_str(fs::read_to_string(&audit).unwrap().trim()).unwrap();
        assert_eq!(entry["outcome"], "cancelled");
        assert_eq!(entry["counts"]["unprocessed"], 1);
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "synthetic.docx")
            .await;
        fs::remove_file(&audit).unwrap();
        fs::create_dir(&audit).unwrap();
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into()],
            Some(&destination),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        assert_eq!(result.exported, ["doc-0001"]);
        assert!(result.audit_warning);
        assert!(result.error.is_none());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
        sidecar.shutdown().await;
    });
}

use std::{cell::RefCell, path::Path};
type LockHook = Box<dyn FnOnce(&Path)>;
type FileHook = Box<dyn FnMut(&str, &Path)>;
thread_local! {
    static AFTER_SNAPSHOT: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static AFTER_VALIDATION: RefCell<Option<LockHook>> = const { RefCell::new(None) };
    static AFTER_LOCK: RefCell<Option<LockHook>> = const { RefCell::new(None) };
    static BEFORE_FILE: RefCell<Option<FileHook>> = const { RefCell::new(None) };
}
pub(super) fn after_snapshot() {
    AFTER_SNAPSHOT.with_borrow_mut(|hook| {
        if let Some(hook) = hook.take() {
            hook();
        }
    });
}
pub(super) fn after_validation(path: &Path) {
    AFTER_VALIDATION.with_borrow_mut(|hook| {
        if let Some(hook) = hook.take() {
            hook(path);
        }
    });
}

#[test]
fn destination_replaced_after_emptiness_check_is_not_adopted() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("export");
    let config = temp.path().join("config");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&config).unwrap();
    AFTER_VALIDATION.set(Some(Box::new(|path| {
        fs::rename(path, path.with_file_name("old-export")).unwrap();
        fs::create_dir(path).unwrap();
    })));
    assert_eq!(
        super::Destination::acquire(&destination, &[], &config)
            .err()
            .unwrap()
            .code,
        "path_changed"
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}
pub(super) fn after_lock(path: &Path) {
    AFTER_LOCK.with_borrow_mut(|hook| {
        if let Some(hook) = hook.take() {
            hook(path);
        }
    });
}
pub(super) fn before_file(id: &str, path: &Path) {
    BEFORE_FILE.with_borrow_mut(|hook| {
        if let Some(hook) = hook {
            hook(id, path);
        }
    });
}

#[test]
fn competing_destination_owners_never_share_or_leave_a_successful_marker() {
    use std::sync::{Arc, Barrier};
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("export");
    let config = temp.path().join("config");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&config).unwrap();
    let start = Arc::new(Barrier::new(2));
    let finish = Arc::new(Barrier::new(2));
    let owners = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    let held = super::Destination::acquire(&destination, &[], &config);
                    finish.wait();
                    held.is_ok()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap() as usize)
            .sum::<usize>()
    });
    assert_eq!(owners, 1);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
}

#[test]
fn external_entry_during_lock_acquisition_is_preserved_and_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("export");
    let config = temp.path().join("config");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&config).unwrap();
    AFTER_LOCK.set(Some(Box::new(|path| {
        fs::write(path.join("external"), b"keep").unwrap()
    })));
    let error = super::Destination::acquire(&destination, &[], &config)
        .err()
        .unwrap();
    assert_eq!(error.code, "export_not_empty");
    assert_eq!(fs::read(destination.join("external")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
}

#[test]
fn modified_or_replaced_marker_is_never_removed_by_the_old_owner() {
    for replace in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("export");
        let config = temp.path().join("config");
        fs::create_dir(&destination).unwrap();
        fs::create_dir(&config).unwrap();
        let owner = super::Destination::acquire(&destination, &[], &config).unwrap();
        let marker = destination.join(super::LOCK_FILE);
        if replace {
            fs::rename(&marker, destination.join("old-marker")).unwrap();
        }
        fs::write(&marker, b"another-owner").unwrap();
        assert_eq!(owner.validate().unwrap_err().code, "path_changed");
        assert_eq!(owner.close().unwrap_err().code, "path_changed");
        assert_eq!(fs::read(marker).unwrap(), b"another-owner");
    }
}

#[test]
fn external_later_file_preserves_completed_export_and_reports_exact_failure() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "first.docx").await;
        f.seed(&sidecar, ReviewStatus::Approved, "second.docx")
            .await;
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        BEFORE_FILE.set(Some(Box::new(|id, path| {
            if id == "doc-0002" {
                fs::write(path.join("doc-0002.md"), b"external bytes").unwrap();
            }
        })));
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into(), "doc-0002".into()],
            Some(&destination),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        BEFORE_FILE.set(None);
        assert_eq!(result.exported, ["doc-0001"]);
        assert_eq!(result.failed[0].doc_id, "doc-0002");
        assert_eq!(result.failed[0].error.code, "path_exists");
        assert_eq!(
            fs::read(destination.join("doc-0001.md")).unwrap(),
            b"previously approved synthetic bytes"
        );
        assert_eq!(
            fs::read(destination.join("doc-0002.md")).unwrap(),
            b"external bytes"
        );
        assert_eq!(fs::read_dir(destination).unwrap().count(), 2);
        sidecar.shutdown().await;
    });
}

#[test]
fn post_publication_error_retains_bytes_but_never_claims_durable_success() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "first.docx").await;
        f.seed(&sidecar, ReviewStatus::Approved, "second.docx")
            .await;
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        crate::domain::storage::tests::FAIL_AFTER_WRITE.set(Some((
            destination.canonicalize().unwrap().join("doc-0002.md"),
            "storage_durability_uncertain",
            false,
        )));
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into(), "doc-0002".into()],
            Some(&destination),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        assert_eq!(result.exported, ["doc-0001"]);
        assert_eq!(result.failed[0].doc_id, "doc-0002");
        assert_eq!(result.failed[0].error.code, "storage_durability_uncertain");
        assert_eq!(
            fs::read(destination.join("doc-0002.md")).unwrap(),
            b"previously approved synthetic bytes"
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
        let retry = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0002".into()],
            Some(&destination),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        assert_eq!(retry.error.unwrap().code, "export_not_empty");
        sidecar.shutdown().await;
    });
}

#[cfg(unix)]
#[test]
fn locked_destination_mid_batch_keeps_completed_files_and_reports_cleanup_failure() {
    use std::os::unix::fs::PermissionsExt;
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("detection");
        f.seed(&sidecar, ReviewStatus::Approved, "first.docx").await;
        f.seed(&sidecar, ReviewStatus::Approved, "second.docx")
            .await;
        let destination = f.root.path().join("export");
        fs::create_dir(&destination).unwrap();
        BEFORE_FILE.set(Some(Box::new(|id, path| {
            if id == "doc-0002" {
                fs::set_permissions(path, fs::Permissions::from_mode(0o500)).unwrap();
            }
        })));
        let result = export_approved(
            &f.controller,
            f.key.sync_pair_id,
            vec!["doc-0001".into(), "doc-0002".into()],
            Some(&destination),
            || Ok(sidecar.clone()),
        )
        .await
        .unwrap();
        BEFORE_FILE.set(None);
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result.exported, ["doc-0001"]);
        assert_eq!(result.failed[0].error.code, "permission_denied");
        assert_eq!(result.error.unwrap().code, "permission_denied");
        assert_eq!(
            fs::read(destination.join("doc-0001.md")).unwrap(),
            b"previously approved synthetic bytes"
        );
        assert!(destination.join(super::LOCK_FILE).exists());
        sidecar.shutdown().await;
    });
}

#[test]
fn destination_identity_change_is_rejected_without_touching_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("export");
    let previous = temp.path().join("old-export");
    let config = temp.path().join("config");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&config).unwrap();
    let owner = super::Destination::acquire(&destination, &[], &config).unwrap();
    fs::rename(&destination, &previous).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join(super::LOCK_FILE), b"other-owner").unwrap();
    assert_eq!(owner.validate().unwrap_err().code, "path_changed");
    assert_eq!(owner.close().unwrap_err().code, "path_changed");
    assert_eq!(
        fs::read(destination.join(super::LOCK_FILE)).unwrap(),
        b"other-owner"
    );
    assert!(previous.join(super::LOCK_FILE).exists());
}

#[test]
fn snapshot_recheck_rejects_source_output_and_destination_races_under_all_guards() {
    runtime().block_on(async {
        for changed in ["source", "output", "destination", "marker"] {
            let f = Fixture::new();
            let sidecar = f.sidecar("detection");
            f.seed(&sidecar, ReviewStatus::Approved, "synthetic.docx")
                .await;
            let destination = f.root.path().join("export");
            fs::create_dir(&destination).unwrap();
            let pair = load_settings(&f.path).unwrap().sync_pairs.remove(0);
            let mutation = match changed {
                "source" => pair.source_folder.join("synthetic.docx"),
                "output" => pair.target_folder.join("doc-0001.md"),
                "marker" => destination.join(super::LOCK_FILE),
                _ => destination.join("doc-0001.md"),
            };
            let controller = f.controller.clone();
            let config = f.path.parent().unwrap().to_path_buf();
            let source = pair.source_folder.clone();
            let target = mutation.clone();
            AFTER_SNAPSHOT.set(Some(Box::new(move || {
                assert_eq!(
                    controller.try_operation().err().unwrap().code,
                    "operation_busy"
                );
                assert!(CollectionGuard::acquire(&config).is_err());
                assert!(CollectionGuard::acquire(&source).is_err());
                fs::write(target, b"external replacement").unwrap();
            })));
            let result = export_approved(
                &f.controller,
                pair.id,
                vec!["doc-0001".into()],
                Some(&destination),
                || Ok(sidecar.clone()),
            )
            .await
            .unwrap();
            assert!(result.exported.is_empty(), "{changed}");
            assert_eq!(result.failed[0].error.code, "path_changed", "{changed}");
            assert_eq!(fs::read(&mutation).unwrap(), b"external replacement");
            if changed == "marker" {
                assert_eq!(result.error.unwrap().code, "path_changed");
            } else {
                assert!(result.error.is_none());
            }
            if changed == "source" || changed == "output" {
                assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            }
            assert!(f.controller.try_operation().is_ok());
            sidecar.shutdown().await;
        }
    });
}
