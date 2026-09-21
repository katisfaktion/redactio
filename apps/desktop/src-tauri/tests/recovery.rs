use redactio_lib::{
    domain::{
        mapping::{
            commit_generation, recover_pending, CollectionGuard, CommitCandidate, Mapping,
            MappingData, PendingCommit, ReviewRecord,
        },
        settings::{save_settings, Settings},
    },
    protocol::{Decisions, DocumentKey, EngineInfo, ReviewStatus},
};
use sha2::{Digest, Sha256};
use std::fs;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn durable_generation_binds_private_review_and_preserves_first_success() {
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
    let pair = &settings.sync_pairs[0];
    let settings_path = config.join("settings.json");
    save_settings(&settings_path, &settings).unwrap();
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
        notes: "private note".into(),
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
        key: key.clone(),
        source_hash: review.source_hash.clone(),
        revision: pair.processing_revision,
        markdown: b"pending markdown".to_vec(),
        review,
    };
    let first = commit_generation(
        pair,
        &mut mapping,
        candidate.clone(),
        None,
        &guard,
        &settings_path,
        &config_guard,
    )
    .unwrap();
    assert_eq!(
        first.output_hash,
        hash(&fs::read(target.join("doc-0001.md")).unwrap())
    );
    let review_path = source.join("_redactio/reviews/doc-0001.json");
    let bytes = fs::read(&review_path).unwrap();
    assert_eq!(first.review_hash, hash(&bytes));
    assert!(!String::from_utf8(bytes).unwrap().contains("original_text"));
    let mut approved = candidate;
    approved.markdown = b"approved markdown".to_vec();
    approved.review.output_hash = hash(&approved.markdown);
    approved.review.status = ReviewStatus::Approved;
    approved.review.reviewed_at = Some("2026-09-19T13:00:00Z".into());
    let saved = commit_generation(
        pair,
        &mut mapping,
        approved.clone(),
        Some(&first.output_hash),
        &guard,
        &settings_path,
        &config_guard,
    )
    .unwrap();
    assert_eq!(saved.first_processed_at, first.first_processed_at);
    assert_eq!(saved.last_processed_at, "2026-09-19T13:00:00Z");
    fs::write(target.join("doc-0001.md"), b"external edit").unwrap();
    assert!(commit_generation(
        pair,
        &mut mapping,
        approved,
        Some(&saved.output_hash),
        &guard,
        &settings_path,
        &config_guard
    )
    .is_err());
    recover_pending(pair, &mut mapping, &guard, &settings_path, &config_guard).unwrap();
    assert_eq!(
        fs::read(target.join("doc-0001.md")).unwrap(),
        b"external edit"
    );
    assert_eq!(
        mapping.reserve_with_guard("b.docx", &guard).unwrap(),
        "doc-0002"
    );
}

#[test]
fn public_recovery_reconciles_a_persisted_partial_publication() {
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
    let pair = &settings.sync_pairs[0];
    let settings_path = config.join("settings.json");
    save_settings(&settings_path, &settings).unwrap();
    let config_guard = CollectionGuard::acquire(&config).unwrap();
    let guard = CollectionGuard::acquire(&source).unwrap();
    let mut mapping = Mapping::load(&source, id, &target).unwrap();
    mapping.reserve_with_guard("a.docx", &guard).unwrap();
    let key = DocumentKey {
        sync_pair_id: id,
        doc_id: "doc-0001".into(),
    };
    let review: ReviewRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "key": key, "source_hash": hash(b"synthetic source"), "revision": pair.processing_revision,
        "detections": [], "decisions": {"dismissed_ids":[],"manual":[]}, "status": "approved", "notes": "private",
        "acknowledged_warnings": [], "warnings": [], "redacted_at": "2026-09-19T12:00:00Z", "reviewed_at": "2026-09-19T13:00:00Z",
        "engine": {"engine_version":"1", "model_name":"synthetic", "model_version":"1", "recognizers":[], "extraction_version":"1"},
        "output_hash": hash(b"approved markdown")
    })).unwrap();
    let review_bytes = serde_json::to_vec_pretty(&review).unwrap();
    let generation = redactio_lib::domain::mapping::Generation {
        source_hash: review.source_hash.clone(),
        revision: review.revision.to_string(),
        output_hash: review.output_hash.clone(),
        review_hash: hash(&review_bytes),
        first_processed_at: review.redacted_at.clone(),
        last_processed_at: review.reviewed_at.clone().unwrap(),
    };
    let pending = PendingCommit {
        generation: generation.clone(),
        prior_output_hash: None,
        prior_review_hash: None,
        review,
        output_temporary: format!(".redactio-commit-{}.tmp", uuid::Uuid::new_v4()),
        review_temporary: format!(".redactio-commit-{}.tmp", uuid::Uuid::new_v4()),
    };
    fs::create_dir_all(source.join("_redactio/reviews")).unwrap();
    fs::write(target.join("doc-0001.md"), b"approved markdown").unwrap();
    fs::write(
        source
            .join("_redactio/reviews")
            .join(&pending.review_temporary),
        &review_bytes,
    )
    .unwrap();
    let mut disk = MappingData::read(&source).unwrap();
    disk.entries[0].pending = Some(pending.clone());
    fs::write(
        source.join("_document-mapping.json"),
        serde_json::to_vec_pretty(&disk).unwrap(),
    )
    .unwrap();
    assert_eq!(
        redactio_lib::domain::scan::scan_collection(pair)
            .unwrap()
            .files[0]
            .state,
        redactio_lib::domain::mapping::DocumentState::RecoveryPending
    );
    recover_pending(pair, &mut mapping, &guard, &settings_path, &config_guard).unwrap();
    assert!(mapping.entries()[0].pending.is_none());
    assert_eq!(mapping.entries()[0].committed.as_ref(), Some(&generation));
    assert_eq!(
        fs::read(source.join("_redactio/reviews/doc-0001.json")).unwrap(),
        review_bytes
    );
    assert!(!source
        .join("_redactio/reviews")
        .join(&pending.review_temporary)
        .exists());
    assert_eq!(
        mapping.reserve_with_guard("b.docx", &guard).unwrap(),
        "doc-0002"
    );
    // Re-running a recovery never rewrites a successful output or reuses its ID.
    fs::write(target.join("doc-0001.md"), b"external edit").unwrap();
    recover_pending(pair, &mut mapping, &guard, &settings_path, &config_guard).unwrap();
    assert_eq!(
        fs::read(target.join("doc-0001.md")).unwrap(),
        b"external edit"
    );
}
