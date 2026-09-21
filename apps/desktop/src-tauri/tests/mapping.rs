use redactio_lib::domain::{
    mapping::{classify, DocumentState, Generation, Mapping},
    settings::Settings,
};
use std::fs;

fn generation() -> Generation {
    Generation {
        source_hash: "a".repeat(64),
        revision: uuid::Uuid::new_v4().to_string(),
        output_hash: "b".repeat(64),
        review_hash: "c".repeat(64),
        first_processed_at: "2026-09-19T12:00:00Z".into(),
        last_processed_at: "2026-09-19T12:00:00Z".into(),
    }
}

#[test]
fn classification_checks_hash_revision_ownership_and_precedence() {
    let g = generation();
    assert_eq!(
        classify(
            Some(&g.source_hash),
            Some("tampered"),
            &g.revision,
            Some(&g),
            false
        ),
        DocumentState::Conflict
    );
    assert_eq!(
        classify(
            Some(&g.source_hash),
            Some(&g.output_hash),
            "new",
            Some(&g),
            false
        ),
        DocumentState::Stale
    );
    assert_eq!(
        classify(
            Some(&g.source_hash),
            Some(&g.output_hash),
            &g.revision,
            Some(&g),
            false
        ),
        DocumentState::Current
    );
    assert_eq!(
        classify(None, None, &g.revision, Some(&g), true),
        DocumentState::RecoveryPending
    );
    assert_eq!(
        classify(None, Some("tampered"), &g.revision, Some(&g), false),
        DocumentState::MissingSource
    );
    assert_eq!(
        classify(Some(&g.source_hash), None, &g.revision, Some(&g), false),
        DocumentState::MissingOutput
    );
    assert_eq!(
        classify(Some("source"), Some("unowned"), "revision", None, false),
        DocumentState::Conflict
    );
    assert_eq!(
        classify(Some("source"), None, "revision", None, false),
        DocumentState::New
    );
}

#[test]
fn reservations_survive_reload_deletion_and_independent_pairs() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let mut settings = Settings::default();
    let id = settings.add("pair", &source, &target).unwrap();
    let mut mapping = Mapping::load(&source, id, &target).unwrap();
    assert_eq!(mapping.reserve("nested/a.docx").unwrap(), "doc-0001");
    let mut reloaded = Mapping::load(&source, id, &target).unwrap();
    assert_eq!(reloaded.reserve("nested/a.docx").unwrap(), "doc-0001");
    assert_eq!(mapping.reserve("b.docx").unwrap(), "doc-0002");
    assert_eq!(reloaded.reserve("c.docx").unwrap(), "doc-0003");
    assert_eq!(
        Mapping::load(&source, id, &target).unwrap().entries().len(),
        3
    );
    assert!(Mapping::load(&source, uuid::Uuid::new_v4(), &target).is_err());
    assert!(mapping.reserve("../escape.docx").is_err());
    let other = root.path().join("other");
    fs::create_dir(&other).unwrap();
    assert!(Mapping::load(&source, id, &other).is_err());
    let second_target = root.path().join("second-target");
    fs::create_dir(&second_target).unwrap();
    let second_id = Settings::default()
        .add("independent", &other, &second_target)
        .unwrap();
    assert_eq!(
        Mapping::load(&other, second_id, &second_target)
            .unwrap()
            .reserve("nested/a.docx")
            .unwrap(),
        "doc-0001"
    );
}

#[test]
fn counter_grows_past_four_digits_and_exhaustion_fails_closed() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    let id = settings.add("pair", source.path(), target.path()).unwrap();
    let path = source.path().join("_document-mapping.json");
    let mut data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    data["next_document_number"] = 10000u64.into();
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let mut mapping = Mapping::load(source.path(), id, target.path()).unwrap();
    assert_eq!(mapping.reserve("a.docx").unwrap(), "doc-10000");
    data["next_document_number"] = u64::MAX.into();
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(
        mapping.reserve("b.docx").unwrap_err().code,
        "document_ids_exhausted"
    );
}

#[test]
fn invalid_or_missing_mapping_never_adopts_old_outputs() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    let id = settings.add("pair", source.path(), target.path()).unwrap();
    let path = source.path().join("_document-mapping.json");
    let original = fs::read(&path).unwrap();
    fs::write(target.path().join("doc-0001.md"), b"unknown").unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(
        Mapping::load(source.path(), id, target.path())
            .unwrap_err()
            .code,
        "mapping_missing"
    );
    assert!(Settings::default()
        .add("pair", source.path(), target.path())
        .is_err());
    for bad in [
        serde_json::json!({"doc_id":"doc-0001","relative_path":"../x","reserved_at":"2026-09-19T12:00:00Z","committed":null}),
        serde_json::json!({"doc_id":"doc-18446744073709551616","relative_path":"x.docx","reserved_at":"2026-09-19T12:00:00Z","committed":null}),
    ] {
        let mut data: serde_json::Value = serde_json::from_slice(&original).unwrap();
        data["entries"] = serde_json::json!([bad]);
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(Mapping::load(source.path(), id, target.path()).is_err());
        assert!(Settings::default()
            .add("pair", source.path(), target.path())
            .is_err());
    }
}

#[cfg(unix)]
#[test]
fn failed_reservation_save_does_not_advance_disk_or_poison_retry() {
    use std::os::unix::fs::PermissionsExt;
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let id = Settings::default()
        .add("pair", source.path(), target.path())
        .unwrap();
    let mut mapping = Mapping::load(source.path(), id, target.path()).unwrap();
    fs::set_permissions(source.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let failed = mapping.reserve("a.docx");
    fs::set_permissions(source.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(failed.is_err());
    assert_eq!(mapping.reserve("a.docx").unwrap(), "doc-0001");
    assert_eq!(mapping.reserve("a.docx").unwrap(), "doc-0001");
}

#[test]
fn pair_scan_classifies_reserved_owned_changed_and_deleted_documents() {
    use redactio_lib::domain::scan::scan_collection;
    use sha2::{Digest, Sha256};
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    let id = settings.add("pair", source.path(), target.path()).unwrap();
    let pair = &settings.sync_pairs[0];
    fs::write(source.path().join("a.docx"), b"source").unwrap();
    let mut mapping = Mapping::load(source.path(), id, target.path()).unwrap();
    mapping.reserve("a.docx").unwrap();
    assert_eq!(
        scan_collection(pair).unwrap().files[0].state,
        DocumentState::New
    );
    fs::write(target.path().join("doc-0001.md"), b"output").unwrap();
    assert_eq!(
        scan_collection(pair).unwrap().files[0].state,
        DocumentState::Conflict
    );
    let path = source.path().join("_document-mapping.json");
    let mut data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut g = generation();
    g.source_hash = format!("{:x}", Sha256::digest(b"source"));
    g.output_hash = format!("{:x}", Sha256::digest(b"output"));
    g.revision = pair.processing_revision.to_string();
    let review = redactio_lib::domain::mapping::ReviewRecord {
        schema_version: 1,
        key: redactio_lib::protocol::DocumentKey {
            sync_pair_id: id,
            doc_id: "doc-0001".into(),
        },
        source_hash: g.source_hash.clone(),
        revision: pair.processing_revision,
        output_hash: g.output_hash.clone(),
        detections: vec![],
        decisions: Default::default(),
        status: redactio_lib::protocol::ReviewStatus::Pending,
        notes: String::new(),
        acknowledged_warnings: vec![],
        warnings: vec![],
        redacted_at: "2026-01-01T00:00:00Z".into(),
        reviewed_at: None,
        engine: redactio_lib::protocol::EngineInfo {
            engine_version: "test".into(),
            model_name: "test".into(),
            model_version: "1".into(),
            extraction_version: "1".into(),
            recognizers: vec![],
        },
    };
    let bytes = serde_json::to_vec_pretty(&review).unwrap();
    fs::create_dir_all(source.path().join("_redactio/reviews")).unwrap();
    fs::write(
        source.path().join("_redactio/reviews/doc-0001.json"),
        &bytes,
    )
    .unwrap();
    g.review_hash = format!("{:x}", Sha256::digest(&bytes));
    data["entries"][0]["committed"] = serde_json::to_value(&g).unwrap();
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(
        scan_collection(pair).unwrap().files[0].state,
        DocumentState::Current
    );
    fs::write(target.path().join("doc-0001.md"), b"changed").unwrap();
    assert_eq!(
        scan_collection(pair).unwrap().files[0].state,
        DocumentState::Conflict
    );
    fs::remove_file(source.path().join("a.docx")).unwrap();
    let report = scan_collection(pair).unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].state, DocumentState::MissingSource);
    assert_eq!(report.files[0].source_hash_sha256, None);
    assert_eq!(report.files[0].mtime, None);
}

#[test]
fn a_collection_guard_excludes_other_writers_and_releases_on_drop() {
    use redactio_lib::domain::mapping::CollectionGuard;
    let source = tempfile::tempdir().unwrap();
    let guard = CollectionGuard::acquire(source.path()).unwrap();
    assert_eq!(
        CollectionGuard::acquire(source.path()).unwrap_err().code,
        "file_busy"
    );
    drop(guard);
    assert!(CollectionGuard::acquire(source.path()).is_ok());
}

#[test]
fn an_incomplete_success_record_is_recovery_work_never_current() {
    let mut g = generation();
    g.review_hash.clear();
    assert_eq!(
        classify(
            Some(&g.source_hash),
            Some(&g.output_hash),
            &g.revision,
            Some(&g),
            false
        ),
        DocumentState::RecoveryPending
    );
}

#[cfg(unix)]
#[test]
fn missing_source_precedes_an_unsafe_output_and_no_link_is_followed() {
    use redactio_lib::domain::scan::scan_collection;
    use std::os::unix::fs::symlink;
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    let id = settings.add("pair", source.path(), target.path()).unwrap();
    Mapping::load(source.path(), id, target.path())
        .unwrap()
        .reserve("gone.docx")
        .unwrap();
    fs::write(outside.path().join("secret"), b"outside").unwrap();
    symlink(
        outside.path().join("secret"),
        target.path().join("doc-0001.md"),
    )
    .unwrap();
    let report = scan_collection(&settings.sync_pairs[0]).unwrap();
    assert_eq!(report.files[0].state, DocumentState::MissingSource);
    assert_eq!(report.errors.len(), 1);
}

#[test]
fn a_child_process_cannot_enter_a_held_collection_lock() {
    use redactio_lib::domain::mapping::CollectionGuard;
    let source = tempfile::tempdir().unwrap();
    let guard = CollectionGuard::acquire(source.path()).unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "collection_lock_child"])
        .env("REDACTIO_LOCK_TEST_SOURCE", source.path())
        .output()
        .unwrap();
    assert!(child.status.success(), "child lock check failed");
    drop(guard);
    assert!(CollectionGuard::acquire(source.path()).is_ok());
}

#[test]
fn collection_lock_child() {
    let Some(source) = std::env::var_os("REDACTIO_LOCK_TEST_SOURCE") else {
        return;
    };
    assert_eq!(
        redactio_lib::domain::mapping::CollectionGuard::acquire(std::path::Path::new(&source))
            .unwrap_err()
            .code,
        "file_busy"
    );
}
