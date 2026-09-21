mod common;

use redactio_lib::{
    domain::{
        detection,
        mapping::{commit_generation, CollectionGuard, CommitCandidate, Mapping, ReviewRecord},
        review::{open_configured, save_configured, SaveReview},
        settings::{load_settings, save_settings, Settings},
        sync::RunController,
    },
    protocol::{Decisions, DocumentKey, EngineInfo, ReviewStatus},
    sidecar::Sidecar,
};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Duration};
use uuid::Uuid;

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
        let model_name = common::fixture_model_store(root.path());
        for pair in &mut settings.sync_pairs {
            pair.config.model = model_name.clone();
        }
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
                common::manifest_dir()
                    .join("tests/fake_sidecar.py")
                    .into_os_string(),
                mode.into(),
                self.root.path().join("engine-version").into_os_string(),
            ],
            self.root.path().into(),
        )
    }
    async fn seed_approved(&self, sidecar: &Sidecar) -> SaveReview {
        let settings =
            detection::refresh_processing_config(&self.controller, sidecar, self.key.sync_pair_id)
                .await
                .unwrap();
        let pair = &settings.sync_pairs[0];
        fs::write(pair.source_folder.join("synthetic.docx"), b"synthetic").unwrap();
        let config_guard = CollectionGuard::acquire(self.path.parent().unwrap()).unwrap();
        let source_guard = CollectionGuard::acquire(&pair.source_folder).unwrap();
        let mut mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
        assert_eq!(
            mapping
                .reserve_with_guard("synthetic.docx", &source_guard)
                .unwrap(),
            self.key.doc_id
        );
        let markdown = b"previously approved synthetic bytes".to_vec();
        let output_hash = format!("{:x}", Sha256::digest(&markdown));
        let source_hash = format!("{:x}", Sha256::digest(b"synthetic"));
        let record = ReviewRecord {
            schema_version: 1,
            key: self.key.clone(),
            source_hash: source_hash.clone(),
            revision: pair.processing_revision,
            detections: vec![],
            decisions: Decisions::default(),
            status: ReviewStatus::Approved,
            notes: "PRIVATE_CANARY".into(),
            acknowledged_warnings: vec![],
            warnings: vec![],
            redacted_at: "2026-01-01T00:00:00Z".into(),
            reviewed_at: Some("2026-01-02T00:00:00Z".into()),
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
                key: self.key.clone(),
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
        SaveReview {
            expected_output_hash: output_hash,
            expected_review_hash: mapping.entries()[0]
                .committed
                .as_ref()
                .unwrap()
                .review_hash
                .clone(),
            decisions: Decisions::default(),
            status: ReviewStatus::Approved,
            notes: String::new(),
            acknowledged_warnings: vec![],
        }
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn direct_open_and_save_refresh_engine_before_accepting_stored_approval() {
    runtime().block_on(async {
        for save in [false, true] {
            let f = Fixture::new();
            let sidecar = f.sidecar("detection");
            let input = f.seed_approved(&sidecar).await;
            let before = load_settings(&f.path).unwrap();
            fs::write(f.root.path().join("engine-version"), "upgraded-engine").unwrap();
            let result = if save {
                save_configured(&f.controller, &sidecar, &f.key, input).await
            } else {
                open_configured(&f.controller, &sidecar, &f.key).await
            };
            assert_eq!(result.unwrap_err().code, "reprocess_required");
            let after = load_settings(&f.path).unwrap();
            assert_ne!(
                after.sync_pairs[0].processing_revision,
                before.sync_pairs[0].processing_revision
            );
            assert_eq!(after.sync_pairs[1], before.sync_pairs[1]);
            assert_eq!(
                after.sync_pairs[0]
                    .processing_fingerprint
                    .as_ref()
                    .unwrap()
                    .engine_version,
                "upgraded-engine"
            );
            let pair = &after.sync_pairs[0];
            assert_eq!(
                fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
                b"previously approved synthetic bytes"
            );
            let mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
            assert_eq!(
                ReviewRecord::read(
                    &pair.source_folder,
                    &f.key,
                    mapping.entries()[0].committed.as_ref().unwrap()
                )
                .unwrap()
                .status,
                ReviewStatus::Approved
            );
            assert_eq!(
                open_configured(&f.controller, &sidecar, &f.key)
                    .await
                    .unwrap_err()
                    .code,
                "reprocess_required"
            );
            assert_eq!(
                load_settings(&f.path).unwrap(),
                after,
                "upgrade rotates only once"
            );
            sidecar.shutdown().await;
        }
    });
}

#[test]
fn direct_review_rejects_changed_revision_and_unknown_identity() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("detection");
        let input = f.seed_approved(&sidecar).await;
        let mut settings = load_settings(&f.path).unwrap();
        settings.sync_pairs[0].processing_revision = Uuid::new_v4();
        save_settings(&f.path, &settings).unwrap();
        assert_eq!(
            save_configured(&f.controller, &sidecar, &f.key, input)
                .await
                .unwrap_err()
                .code,
            "reprocess_required"
        );
        assert_eq!(
            open_configured(&f.controller, &sidecar, &f.key)
                .await
                .unwrap_err()
                .code,
            "reprocess_required"
        );
        let foreign = DocumentKey {
            sync_pair_id: Uuid::new_v4(),
            doc_id: "doc-0001".into(),
        };
        assert_eq!(
            open_configured(&f.controller, &sidecar, &foreign)
                .await
                .unwrap_err()
                .code,
            "unknown_pair"
        );
        let missing = DocumentKey {
            doc_id: "doc-9999".into(),
            ..f.key.clone()
        };
        assert_eq!(
            open_configured(&f.controller, &sidecar, &missing)
                .await
                .unwrap_err()
                .code,
            "unknown_document"
        );
        assert!(f.controller.try_operation().is_ok());
        sidecar.shutdown().await;
    });
}

#[test]
fn review_render_retains_app_config_source_guards_until_abort() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar("batch-review-pause");
        let input = f.seed_approved(&sidecar).await;
        let controller = f.controller.clone();
        let child = sidecar.clone();
        let key = f.key.clone();
        let worker = tokio::spawn(async move { open_configured(&controller, &child, &key).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !f.root.path().join("engine-version").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            f.controller.try_operation().err().unwrap().code,
            "operation_busy"
        );
        assert_eq!(
            save_configured(&f.controller, &sidecar, &f.key, input)
                .await
                .unwrap_err()
                .code,
            "operation_busy"
        );
        let pair = load_settings(&f.path).unwrap().sync_pairs.remove(0);
        assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_err());
        assert!(CollectionGuard::acquire(&pair.source_folder).is_err());
        let other_app = RunController::new(f.path.clone());
        assert_eq!(
            open_configured(&other_app, &sidecar, &f.key)
                .await
                .unwrap_err()
                .code,
            "file_busy"
        );
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        assert!(f.controller.try_operation().is_ok());
        assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_ok());
        assert!(CollectionGuard::acquire(&pair.source_folder).is_ok());
        assert_eq!(
            fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
            b"previously approved synthetic bytes"
        );
        sidecar.shutdown().await;
    });
}
