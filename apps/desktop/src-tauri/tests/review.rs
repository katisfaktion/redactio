use redactio_lib::{
    domain::review::{is_current_approval, ApprovalBinding},
    protocol::DocumentKey,
};

#[test]
fn approval_never_transfers_to_new_bytes_or_another_pair() {
    let saved = ApprovalBinding {
        key: DocumentKey {
            sync_pair_id: uuid::Uuid::new_v4(),
            doc_id: "doc-0001".into(),
        },
        source_hash: "a".repeat(64),
        revision: uuid::Uuid::new_v4().to_string(),
        output_hash: "b".repeat(64),
    };
    for field in 0..5 {
        let mut changed = saved.clone();
        match field {
            0 => changed.output_hash = "c".repeat(64),
            1 => changed.key.sync_pair_id = uuid::Uuid::new_v4(),
            2 => changed.key.doc_id = "doc-0002".into(),
            3 => changed.source_hash = "d".repeat(64),
            _ => changed.revision = uuid::Uuid::new_v4().to_string(),
        }
        assert!(!is_current_approval(&saved, &changed));
    }
    assert!(is_current_approval(&saved, &saved));
}

use redactio_lib::{
    domain::{
        mapping::{
            commit_generation, CollectionGuard, CommitCandidate, Mapping, MappingData, ReviewRecord,
        },
        review::{open_review, save_review, SaveReview},
        settings::{save_settings, EntityType, Settings, SyncPair},
        sync::configure_pair,
    },
    protocol::{
        Decisions, Detection, DetectionOrigin, EngineInfo, ProcessRequest, ProcessResult,
        ReviewStatus,
    },
    sidecar::{Sidecar, DOCUMENT_TIMEOUT},
};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Fixture {
    _root: tempfile::TempDir,
    path: PathBuf,
    pair: SyncPair,
    config_guard: CollectionGuard,
    guard: CollectionGuard,
    sidecar: Sidecar,
    engine: EngineInfo,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let source = root.path().join("source");
        let target = root.path().join("target");
        for p in [&config, &source, &target] {
            fs::create_dir(p).unwrap();
        }
        let mut settings = Settings::default();
        settings.add("Synthetic", &source, &target).unwrap();
        settings.sync_pairs[0].config.enabled_entities = vec![EntityType::EmailAddress];
        let pair = settings.sync_pairs[0].clone();
        let path = config.join("settings.json");
        save_settings(&path, &settings).unwrap();
        let python = PathBuf::from(
            std::env::var_os("REDACTIO_REAL_SIDECAR_PYTHON").expect("reviewed Python required"),
        );
        let status = std::process::Command::new(&python).args(["-c", r#"
import sys
from pathlib import Path
from docx import Document
root = Path(sys.argv[1])
for name, text in [('normal', '😀 Kontakt anna@example.org in Berlin.'), ('warning', 'Text ohne Kennung'), ('empty', '')]:
    doc = Document()
    if text: doc.add_paragraph(text)
    if name == 'warning': doc.sections[0].header.paragraphs[0].text = 'Private header'
    doc.save(root / (name + '.docx'))
"#]).arg(&source).status().unwrap();
        assert!(status.success());
        let sidecar = Sidecar::new(
            python,
            // Real CLI/engine/extractor/renderer; tripwires forbid analysis during
            // review and a second process_document for any already seeded key.
            vec![
                "-c".into(),
                r#"
from redactio_sidecar.engine import Engine
from redactio_sidecar.ipc import main
analyze, process = Engine.analyze, Engine.process_document
seen = set()
def checked_analyze(self, *args):
    if not getattr(self, '_test_seeding', False):
        raise AssertionError('review reran analysis')
    return analyze(self, *args)
def seed_once(self, request):
    key = (request.sync_pair_id, request.doc_id)
    if key in seen:
        raise AssertionError('review reprocessed source')
    seen.add(key)
    self._test_seeding = True
    try:
        return process(self, request)
    finally:
        self._test_seeding = False
Engine.analyze = checked_analyze
Engine.process_document = seed_once
main()
"#
                .into(),
            ],
            std::env::var_os("REDACTIO_REAL_MODEL_DIR")
                .expect("models required")
                .into(),
        );
        let settings = redactio_lib::domain::detection::refresh_processing_config(
            &redactio_lib::domain::sync::RunController::new(path.clone()),
            &sidecar,
            pair.id,
        )
        .await
        .unwrap();
        let pair = settings.sync_pairs[0].clone();
        let config_guard = CollectionGuard::acquire(&config).unwrap();
        let guard = CollectionGuard::acquire(&source).unwrap();
        let engine = configure_pair(&pair, &sidecar).await.unwrap().engine;
        Self {
            _root: root,
            path,
            pair,
            config_guard,
            guard,
            sidecar,
            engine,
        }
    }
    async fn seed(&self, name: &str) -> DocumentKey {
        let mut mapping = Mapping::load(
            &self.pair.source_folder,
            self.pair.id,
            &self.pair.target_folder,
        )
        .unwrap();
        let filename = format!("{name}.docx");
        let doc_id = mapping.reserve_with_guard(&filename, &self.guard).unwrap();
        let source_path = self.pair.source_folder.join(filename);
        let key = DocumentKey {
            sync_pair_id: self.pair.id,
            doc_id,
        };
        let request = ProcessRequest {
            sync_pair_id: key.sync_pair_id,
            doc_id: key.doc_id.clone(),
            source_hash_sha256: digest(&fs::read(&source_path).unwrap()),
            processing_revision: self.pair.processing_revision,
            redacted_at: "2026-01-01T00:00:00Z".into(),
            source_path: source_path.to_str().unwrap().into(),
        };
        let result: ProcessResult = self
            .sidecar
            .request("process_document", &request, DOCUMENT_TIMEOUT)
            .await
            .unwrap();
        let review = ReviewRecord {
            schema_version: 1,
            key: key.clone(),
            source_hash: request.source_hash_sha256.clone(),
            revision: self.pair.processing_revision,
            detections: result.detections,
            decisions: Decisions::default(),
            status: result.review_status,
            notes: String::new(),
            acknowledged_warnings: vec![],
            warnings: result.warnings,
            redacted_at: request.redacted_at,
            reviewed_at: None,
            engine: result.engine,
            output_hash: digest(result.markdown.as_bytes()),
        };
        commit_generation(
            &self.pair,
            &mut mapping,
            CommitCandidate {
                key: key.clone(),
                source_hash: request.source_hash_sha256,
                revision: self.pair.processing_revision,
                markdown: result.markdown.into_bytes(),
                review,
            },
            None,
            &self.guard,
            &self.path,
            &self.config_guard,
        )
        .unwrap();
        key
    }
    async fn open(
        &self,
        key: &DocumentKey,
    ) -> Result<redactio_lib::domain::review::ReviewViewData, redactio_lib::error::AppError> {
        open_review(
            &self.pair,
            key,
            &self.path,
            &self.config_guard,
            &self.guard,
            &self.sidecar,
            &self.engine,
        )
        .await
    }
    async fn save(
        &self,
        key: &DocumentKey,
        input: SaveReview,
    ) -> Result<redactio_lib::domain::review::ReviewViewData, redactio_lib::error::AppError> {
        save_review(
            &self.pair,
            key,
            input,
            &self.path,
            &self.config_guard,
            &self.guard,
            &self.sidecar,
            &self.engine,
        )
        .await
    }
    fn record(&self, key: &DocumentKey) -> ReviewRecord {
        let data = MappingData::read(&self.pair.source_folder).unwrap();
        ReviewRecord::read(
            &self.pair.source_folder,
            key,
            data.entries
                .iter()
                .find(|e| e.doc_id == key.doc_id)
                .unwrap()
                .committed
                .as_ref()
                .unwrap(),
        )
        .unwrap()
    }
}

fn input(view: &redactio_lib::domain::review::ReviewViewData, status: ReviewStatus) -> SaveReview {
    SaveReview {
        expected_output_hash: view.expected_output_hash.clone(),
        decisions: view.decisions.clone(),
        status,
        notes: "PRIVATE_NOTE_CANARY".into(),
        acknowledged_warnings: view.acknowledged_warnings.clone(),
    }
}

#[test]
#[ignore = "requires the reviewed sidecar interpreter and bundled models"]
fn real_review_corrections_approval_and_tampering() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let f = Fixture::new().await;
            let key = f.seed("normal").await;
            let source_before = fs::read(f.pair.source_folder.join("normal.docx")).unwrap();
            let initial = f.open(&key).await.unwrap();
            assert_eq!(initial.detections.len(), 1);
            assert!(initial.body.contains("<EMAIL_ADDRESS_1>"));
            let approved = f
                .save(&key, input(&initial, ReviewStatus::Approved))
                .await
                .unwrap();
            assert_eq!(approved.status, ReviewStatus::Approved);
            assert_eq!(f.record(&key).redacted_at, "2026-01-01T00:00:00Z");
            assert!(f.record(&key).reviewed_at.is_some());
            assert!(!approved.markdown.contains("PRIVATE_NOTE_CANARY"));
            assert_eq!(
                digest(approved.markdown.as_bytes()),
                approved.expected_output_hash
            );
            let mut correction = input(&approved, ReviewStatus::Approved);
            correction
                .decisions
                .dismissed_ids
                .push(initial.detections[0].id.clone());
            correction.decisions.manual.push(Detection {
                id: "manual-type-change".into(),
                start: initial.detections[0].start,
                end: initial.detections[0].end,
                entity_type: EntityType::Person,
                confidence: None,
                recognizer: "manual".into(),
                origin: DetectionOrigin::Manual,
            });
            let berlin = initial
                .original_text
                .chars()
                .collect::<Vec<_>>()
                .windows(6)
                .position(|w| w.iter().collect::<String>() == "Berlin")
                .unwrap() as u64;
            correction.decisions.manual.push(Detection {
                id: "manual-add".into(),
                start: berlin,
                end: berlin + 6,
                entity_type: EntityType::Location,
                confidence: None,
                recognizer: "manual".into(),
                origin: DetectionOrigin::Manual,
            });
            let edited = f.save(&key, correction).await.unwrap();
            assert_eq!(
                edited.status,
                ReviewStatus::Pending,
                "edits cannot simultaneously approve"
            );
            assert!(edited.body.contains("<PERSON_1>"));
            assert!(edited.body.contains("<LOCATION_1>"));
            assert!(!edited.body.contains("anna@example.org"));
            assert!(f.record(&key).reviewed_at.is_none());
            assert_ne!(f.record(&key).redacted_at, "2026-01-01T00:00:00Z");
            let reopened = f.open(&key).await.unwrap();
            assert_eq!(reopened.decisions, edited.decisions);
            assert_eq!(reopened.detections, initial.detections);
            assert_eq!(reopened.notes, "PRIVATE_NOTE_CANARY");
            assert_eq!(reopened.markdown, edited.markdown);
            assert_eq!(
                f.save(&key, input(&approved, ReviewStatus::Approved))
                    .await
                    .unwrap_err()
                    .code,
                "output_conflict"
            );
            for mutation in 0..5 {
                let mut invalid = input(&reopened, ReviewStatus::Pending);
                match mutation {
                    0 => invalid.decisions.manual[0].end = 1_000_001,
                    1 => invalid.decisions.manual[0].origin = DetectionOrigin::Automatic,
                    2 => invalid.decisions.dismissed_ids.push("unknown-id".into()),
                    3 => invalid.decisions.manual[0].id = initial.detections[0].id.clone(),
                    _ => invalid.decisions.manual[0].confidence = Some(0.9),
                }
                assert_eq!(
                    f.save(&key, invalid).await.unwrap_err().code,
                    "invalid_review"
                );
            }
            let rejected = f
                .save(&key, input(&reopened, ReviewStatus::Rejected))
                .await
                .unwrap();
            assert_eq!(rejected.status, ReviewStatus::Rejected);
            assert!(f.record(&key).reviewed_at.is_some());
            let final_view = f
                .save(&key, input(&rejected, ReviewStatus::Approved))
                .await
                .unwrap();
            assert_eq!(f.open(&key).await.unwrap().status, ReviewStatus::Approved);
            assert_eq!(
                source_before,
                fs::read(f.pair.source_folder.join("normal.docx")).unwrap()
            );
            let warning = f.seed("warning").await;
            let warning_view = f.open(&warning).await.unwrap();
            assert_eq!(warning_view.status, ReviewStatus::NeedsRework);
            assert_eq!(
                f.save(&warning, input(&warning_view, ReviewStatus::Approved))
                    .await
                    .unwrap_err()
                    .code,
                "approval_not_allowed"
            );
            let mut acknowledged = input(&warning_view, ReviewStatus::Approved);
            acknowledged.acknowledged_warnings = warning_view.warnings.clone();
            assert_eq!(
                f.save(&warning, acknowledged).await.unwrap().status,
                ReviewStatus::Approved
            );
            let empty = f.seed("empty").await;
            let empty_view = f.open(&empty).await.unwrap();
            assert_eq!(empty_view.status, ReviewStatus::NeedsRework);
            let mut empty_approval = input(&empty_view, ReviewStatus::Approved);
            empty_approval.acknowledged_warnings = empty_view.warnings.clone();
            assert_eq!(
                f.save(&empty, empty_approval).await.unwrap_err().code,
                "approval_not_allowed"
            );
            assert_eq!(
                f.save(&empty, input(&empty_view, ReviewStatus::Rejected))
                    .await
                    .unwrap()
                    .status,
                ReviewStatus::NeedsRework
            );
            for (path, replacement, expected) in [
                (
                    f.pair.source_folder.join("normal.docx"),
                    b"changed-source".to_vec(),
                    "reprocess_required",
                ),
                (
                    f.pair.target_folder.join(format!("{}.md", key.doc_id)),
                    b"external-output".to_vec(),
                    "output_conflict",
                ),
                (
                    f.pair
                        .source_folder
                        .join(format!("_redactio/reviews/{}.json", key.doc_id)),
                    b"{}".to_vec(),
                    "review_mismatch",
                ),
            ] {
                let original = fs::read(&path).unwrap();
                fs::write(&path, replacement).unwrap();
                assert_eq!(f.open(&key).await.unwrap_err().code, expected);
                assert_eq!(
                    f.save(&key, input(&final_view, ReviewStatus::Approved))
                        .await
                        .unwrap_err()
                        .code,
                    expected
                );
                fs::write(path, original).unwrap();
            }
            let source_path = f.pair.source_folder.join("normal.docx");
            fs::remove_file(&source_path).unwrap();
            assert_eq!(f.open(&key).await.unwrap_err().code, "missing_source");
            fs::write(source_path, source_before).unwrap();
            let original_settings = fs::read(&f.path).unwrap();
            let mut settings: Settings = serde_json::from_slice(&original_settings).unwrap();
            settings.sync_pairs[0].processing_revision = uuid::Uuid::new_v4();
            save_settings(&f.path, &settings).unwrap();
            assert_eq!(f.open(&key).await.unwrap_err().code, "reprocess_required");
            fs::write(&f.path, original_settings).unwrap();
            let mut foreign = key.clone();
            foreign.sync_pair_id = uuid::Uuid::new_v4();
            assert_eq!(
                f.open(&foreign).await.unwrap_err().code,
                "mapping_pair_mismatch"
            );
            let output_path = f.pair.target_folder.join(format!("{}.md", key.doc_id));
            let output_bytes = fs::read(&output_path).unwrap();
            fs::remove_file(&output_path).unwrap();
            assert_eq!(f.open(&key).await.unwrap_err().code, "reprocess_required");
            fs::write(&output_path, output_bytes).unwrap();
            let mapping_path = f.pair.source_folder.join("_document-mapping.json");
            let original_mapping = fs::read(&mapping_path).unwrap();
            let mut pending_data = MappingData::read(&f.pair.source_folder).unwrap();
            let generation = pending_data.entries[0].committed.clone().unwrap();
            pending_data.entries[0].pending = Some(redactio_lib::domain::mapping::PendingCommit {
                generation: generation.clone(),
                prior_output_hash: Some(generation.output_hash.clone()),
                prior_review_hash: Some(generation.review_hash),
                review: f.record(&key),
                output_temporary: format!(".redactio-commit-{}.tmp", uuid::Uuid::new_v4()),
                review_temporary: format!(".redactio-commit-{}.tmp", uuid::Uuid::new_v4()),
            });
            fs::write(
                &mapping_path,
                serde_json::to_vec_pretty(&pending_data).unwrap(),
            )
            .unwrap();
            assert_eq!(f.open(&key).await.unwrap_err().code, "recovery_pending");
            assert_eq!(
                f.save(&key, input(&final_view, ReviewStatus::Approved))
                    .await
                    .unwrap_err()
                    .code,
                "recovery_pending"
            );
            fs::write(&mapping_path, &original_mapping).unwrap();
            // Rehash malicious private spans to prove validation is not checksum-only.
            let review_path = f
                .pair
                .source_folder
                .join(format!("_redactio/reviews/{}.json", key.doc_id));
            let original_review = fs::read(&review_path).unwrap();
            for automatic in [false, true] {
                let mut record: ReviewRecord = serde_json::from_slice(&original_review).unwrap();
                if automatic {
                    record.detections[0].start += 1;
                } else {
                    record.decisions.manual[0].end = 1000;
                }
                let bytes = serde_json::to_vec_pretty(&record).unwrap();
                fs::write(&review_path, &bytes).unwrap();
                let mut mapping: MappingData = serde_json::from_slice(&original_mapping).unwrap();
                mapping.entries[0].committed.as_mut().unwrap().review_hash = digest(&bytes);
                fs::write(&mapping_path, serde_json::to_vec_pretty(&mapping).unwrap()).unwrap();
                assert_eq!(f.open(&key).await.unwrap_err().code, "invalid_review");
            }
            assert!(!f.path.parent().unwrap().join("audit-log.jsonl").exists());
            f.sidecar.shutdown().await;
        });
}

#[test]
#[ignore = "requires the reviewed sidecar interpreter and bundled models"]
fn real_configured_commands_approve_reopen_and_reject_changed_settings() {
    use redactio_lib::domain::{
        detection,
        review::{open_configured, save_configured},
        settings::load_settings,
        sync::RunController,
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let f = Fixture::new().await;
            let key = f.seed("normal").await;
            let Fixture {
                _root,
                path,
                pair,
                config_guard,
                guard,
                sidecar,
                ..
            } = f;
            drop(guard);
            drop(config_guard);
            let controller = RunController::new(path.clone());
            let settings_bytes = fs::read(&path).unwrap();
            let opened = open_configured(&controller, &sidecar, &key).await.unwrap();
            assert_eq!(opened.status, ReviewStatus::Pending);
            assert!(opened.body.contains("<EMAIL_ADDRESS_1>"));
            let approved = save_configured(
                &controller,
                &sidecar,
                &key,
                input(&opened, ReviewStatus::Approved),
            )
            .await
            .unwrap();
            assert_eq!(approved.status, ReviewStatus::Approved);
            assert_eq!(
                digest(approved.markdown.as_bytes()),
                approved.expected_output_hash
            );
            assert!(!approved.markdown.contains("PRIVATE_NOTE_CANARY"));
            let reopened = open_configured(&controller, &sidecar, &key).await.unwrap();
            assert_eq!(reopened.status, ReviewStatus::Approved);
            assert_eq!(reopened.expected_output_hash, approved.expected_output_hash);
            assert_eq!(
                fs::read(&path).unwrap(),
                settings_bytes,
                "review preserves equivalent saved configuration"
            );
            let output_bytes = fs::read(pair.target_folder.join("doc-0001.md")).unwrap();
            let mut changed = pair.config.clone();
            changed.include_positions = false;
            detection::save_processing_config(&controller, &sidecar, pair.id, changed)
                .await
                .unwrap();
            assert_ne!(
                load_settings(&path).unwrap().sync_pairs[0].processing_revision,
                pair.processing_revision
            );
            assert_eq!(
                open_configured(&controller, &sidecar, &key)
                    .await
                    .unwrap_err()
                    .code,
                "reprocess_required"
            );
            assert_eq!(
                save_configured(
                    &controller,
                    &sidecar,
                    &key,
                    input(&approved, ReviewStatus::Approved)
                )
                .await
                .unwrap_err()
                .code,
                "reprocess_required"
            );
            assert_eq!(
                fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
                output_bytes
            );
            assert!(controller.try_operation().is_ok());
            sidecar.shutdown().await;
        });
}
