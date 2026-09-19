use redactio_lib::domain::{
    mapping::Mapping,
    recovery::{fresh_start, recovery_pairs},
    settings::{load_settings, save_settings, Settings},
};
use std::fs;

#[test]
fn fresh_start_is_explicit_preserves_originals_old_outputs_and_private_backup() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    let fresh = root.path().join("fresh");
    let config = root.path().join("config");
    for dir in [&source, &target, &fresh, &config] {
        fs::create_dir(dir).unwrap();
    }
    let settings_path = config.join("settings.json");
    let mut settings = Settings::default();
    let old_id = settings.add("pair", &source, &target).unwrap();
    save_settings(&settings_path, &settings).unwrap();
    fs::write(source.join("a.docx"), b"original").unwrap();
    fs::write(target.join("doc-0001.md"), b"old output").unwrap();
    fs::create_dir_all(source.join("_redactio/reviews")).unwrap();
    fs::write(
        source.join("_redactio/reviews/doc-0001.json"),
        b"private review",
    )
    .unwrap();
    fs::write(source.join("_document-mapping.json"), b"broken mapping").unwrap();
    assert!(settings.validate_registry(&[config.clone()]).is_err());
    assert_eq!(recovery_pairs(&settings_path).unwrap().len(), 1);
    assert_eq!(
        fresh_start(&settings_path, old_id, &fresh, false)
            .unwrap_err()
            .code,
        "confirmation_required"
    );
    let repaired = fresh_start(&settings_path, old_id, &fresh, true).unwrap();
    let pair = &repaired.sync_pairs[0];
    assert_ne!(pair.id, old_id);
    assert_ne!(
        pair.processing_revision,
        settings.sync_pairs[0].processing_revision
    );
    repaired.validate_registry(&[config]).unwrap();
    assert_eq!(fs::read(source.join("a.docx")).unwrap(), b"original");
    assert_eq!(fs::read(target.join("doc-0001.md")).unwrap(), b"old output");
    let backup = source.join(format!(".redactio-backup-{}", pair.id));
    assert_eq!(
        fs::read(backup.join("_document-mapping.json")).unwrap(),
        b"broken mapping"
    );
    assert_eq!(
        fs::read(backup.join("_redactio/reviews/doc-0001.json")).unwrap(),
        b"private review"
    );
    assert!(!source.join("_redactio").exists());
    assert_eq!(
        Mapping::load(&source, pair.id, &fresh)
            .unwrap()
            .reserve("a.docx")
            .unwrap(),
        "doc-0001"
    );
    assert_eq!(load_settings(&settings_path).unwrap(), repaired);
}

#[test]
fn recovery_refuses_existing_target_and_healthy_mapping() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let settings_path = config.path().join("settings.json");
    let mut settings = Settings::default();
    let id = settings.add("pair", source.path(), target.path()).unwrap();
    save_settings(&settings_path, &settings).unwrap();
    assert_eq!(
        fresh_start(&settings_path, id, fresh.path(), true)
            .unwrap_err()
            .code,
        "mapping_not_broken"
    );
    fs::remove_file(source.path().join("_document-mapping.json")).unwrap();
    fs::write(fresh.path().join("unknown.md"), b"unknown").unwrap();
    assert_eq!(
        fresh_start(&settings_path, id, fresh.path(), true)
            .unwrap_err()
            .code,
        "target_not_empty"
    );
    assert_eq!(load_settings(&settings_path).unwrap(), settings);
}
