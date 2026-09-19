use redactio_lib::domain::settings::{
    load_settings, save_settings, CustomRule, EntityType, ProcessingConfig, ProcessingFingerprint,
    Settings,
};
use serde_json::{json, Value};
use std::fs;
use uuid::Uuid;

fn folders(root: &tempfile::TempDir, stem: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let source = root.path().join(format!("{stem}-source"));
    let target = root.path().join(format!("{stem}-target"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    (source, target)
}

#[test]
fn remove_keeps_collection_data_and_readd_keeps_identity_with_fresh_defaults() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let mut settings = Settings::default();
    let id = settings.add("Buch", &source, &target).unwrap();
    settings.select(id).unwrap();
    let first_revision = settings.sync_pairs[0].processing_revision;
    fs::write(target.join("doc-0001.md"), "retained").unwrap();
    let path = root.path().join("settings.json");
    save_settings(&path, &settings).unwrap();

    let mut loaded = load_settings(&path).unwrap();
    loaded.remove(id).unwrap();
    assert!(source.join("_document-mapping.json").exists());
    assert_eq!(
        fs::read_to_string(target.join("doc-0001.md")).unwrap(),
        "retained"
    );
    assert_eq!(loaded.add("Buch erneut", &source, &target).unwrap(), id);
    assert_ne!(loaded.sync_pairs[0].processing_revision, first_revision);
    assert_eq!(loaded.sync_pairs[0].config.model, "de_core_news_lg");
    assert_eq!(loaded.sync_pairs[0].config.enabled_entities.len(), 8);
    assert!(loaded.sync_pairs[0].config.custom_rules.is_empty());
    assert!(loaded.sync_pairs[0].config.include_positions);
}

#[test]
fn names_are_trimmed_nonempty_and_unicode_case_insensitively_unique() {
    let root = tempfile::tempdir().unwrap();
    let (source_a, target_a) = folders(&root, "a");
    let (source_b, target_b) = folders(&root, "b");
    let mut settings = Settings::default();

    assert_eq!(
        settings.add("   ", &source_a, &target_a).unwrap_err().code,
        "invalid_pair_name"
    );
    settings.add(" BÜCHER ", &source_a, &target_a).unwrap();
    assert_eq!(settings.sync_pairs[0].name, "BÜCHER");
    assert_eq!(
        settings
            .add("bücher", &source_b, &target_b)
            .unwrap_err()
            .code,
        "duplicate_pair_name"
    );
}

#[test]
fn canonically_equivalent_names_are_duplicates_without_changing_display_text() {
    let root = tempfile::tempdir().unwrap();
    let (source_a, target_a) = folders(&root, "a");
    let (source_b, target_b) = folders(&root, "b");
    let mut settings = Settings::default();

    settings.add("Cafe\u{301}", &source_a, &target_a).unwrap();
    assert_eq!(settings.sync_pairs[0].name, "Cafe\u{301}");
    assert_eq!(
        settings.add("CAFÉ", &source_b, &target_b).unwrap_err().code,
        "duplicate_pair_name"
    );
}

#[test]
fn load_rejects_unknown_fields_invalid_selection_and_duplicate_ids() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    fs::write(&path, r#"{"schema_version":1,"sync_pairs":[],"selected_sync_pair_id":"00000000-0000-0000-0000-000000000001"}"#).unwrap();
    assert_eq!(load_settings(&path).unwrap_err().code, "invalid_settings");

    fs::write(
        &path,
        r#"{"schema_version":1,"sync_pairs":[],"selected_sync_pair_id":null,"extra":true}"#,
    )
    .unwrap();
    assert_eq!(load_settings(&path).unwrap_err().code, "invalid_settings");

    let (source_a, target_a) = folders(&root, "a");
    let (source_b, target_b) = folders(&root, "b");
    let mut settings = Settings::default();
    settings.add("A", &source_a, &target_a).unwrap();
    settings.add("B", &source_b, &target_b).unwrap();
    let mut value = serde_json::to_value(&settings).unwrap();
    value["sync_pairs"][1]["id"] = value["sync_pairs"][0]["id"].clone();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(load_settings(&path).unwrap_err().code, "invalid_settings");
}

#[test]
fn missing_roots_and_wrong_readd_target_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let missing = root.path().join("missing");
    let mut settings = Settings::default();
    assert_eq!(
        settings.add("Missing", &missing, &target).unwrap_err().code,
        "path_unavailable"
    );

    let id = settings.add("Book", &source, &target).unwrap();
    settings.remove(id).unwrap();
    let wrong_target = root.path().join("wrong-target");
    fs::create_dir(&wrong_target).unwrap();
    assert_eq!(
        settings
            .add("Book", &source, &wrong_target)
            .unwrap_err()
            .code,
        "mapping_target_mismatch"
    );
}

#[cfg(unix)]
#[test]
fn readd_compares_the_mapping_target_by_canonical_identity() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let target_alias = root.path().join("target-alias");
    symlink(&target, &target_alias).unwrap();
    let mut settings = Settings::default();
    let id = settings.add("Book", &source, &target).unwrap();
    settings.remove(id).unwrap();

    let mapping_path = source.join("_document-mapping.json");
    let mut mapping: Value = serde_json::from_slice(&fs::read(&mapping_path).unwrap()).unwrap();
    mapping["target_folder"] = json!(target_alias);
    fs::write(&mapping_path, serde_json::to_vec(&mapping).unwrap()).unwrap();

    settings.validate_registry(&[]).unwrap();
    assert_eq!(settings.add("Book again", &source, &target).unwrap(), id);
}

#[test]
fn registry_validation_rejects_a_missing_mapping_without_discarding_settings() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let mut settings = Settings::default();
    let id = settings.add("Book", &source, &target).unwrap();
    fs::remove_file(source.join("_document-mapping.json")).unwrap();

    assert_eq!(
        settings.validate_registry(&[]).unwrap_err().code,
        "mapping_missing"
    );
    assert_eq!(settings.sync_pairs[0].id, id);
}

#[test]
fn registry_validation_rejects_a_corrupt_mapping_without_replacing_it() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let mut settings = Settings::default();
    settings.add("Book", &source, &target).unwrap();
    let mapping_path = source.join("_document-mapping.json");
    fs::write(&mapping_path, b"not json").unwrap();

    assert_eq!(
        settings.validate_registry(&[]).unwrap_err().code,
        "invalid_mapping"
    );
    assert_eq!(fs::read(mapping_path).unwrap(), b"not json");
}

#[test]
fn registry_validation_rejects_a_mismatched_target_binding() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let (_, wrong_target) = folders(&root, "wrong");
    let mut settings = Settings::default();
    settings.add("Book", &source, &target).unwrap();
    let mapping_path = source.join("_document-mapping.json");
    let mut mapping: Value = serde_json::from_slice(&fs::read(&mapping_path).unwrap()).unwrap();
    mapping["target_folder"] = json!(fs::canonicalize(wrong_target).unwrap());
    fs::write(mapping_path, serde_json::to_vec(&mapping).unwrap()).unwrap();

    assert_eq!(
        settings.validate_registry(&[]).unwrap_err().code,
        "mapping_target_mismatch"
    );
}

#[test]
fn registry_validation_rejects_a_mapping_replaced_with_another_pair_id() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let mut settings = Settings::default();
    settings.add("Book", &source, &target).unwrap();
    let mapping_path = source.join("_document-mapping.json");
    let mut mapping: Value = serde_json::from_slice(&fs::read(&mapping_path).unwrap()).unwrap();
    mapping["sync_pair_id"] = json!(Uuid::new_v4());
    fs::write(mapping_path, serde_json::to_vec(&mapping).unwrap()).unwrap();

    assert_eq!(
        settings.validate_registry(&[]).unwrap_err().code,
        "mapping_pair_mismatch"
    );
}

#[test]
fn mapping_uuid_cannot_be_registered_for_two_sources() {
    let root = tempfile::tempdir().unwrap();
    let (source_a, target_a) = folders(&root, "a");
    let (source_b, target_b) = folders(&root, "b");
    let mut settings = Settings::default();
    let id = settings.add("A", &source_a, &target_a).unwrap();
    let mut mapping: Value =
        serde_json::from_slice(&fs::read(source_a.join("_document-mapping.json")).unwrap())
            .unwrap();
    mapping["target_folder"] = json!(fs::canonicalize(&target_b).unwrap());
    fs::write(
        source_b.join("_document-mapping.json"),
        serde_json::to_vec(&mapping).unwrap(),
    )
    .unwrap();

    let error = settings.add("B", &source_b, &target_b).unwrap_err();
    assert_eq!(error.code, "duplicate_pair_id");
    assert_eq!(settings.sync_pairs[0].id, id);
}

#[test]
fn rename_keeps_revision_and_remove_selects_the_adjacent_pair() {
    let root = tempfile::tempdir().unwrap();
    let (source_a, target_a) = folders(&root, "a");
    let (source_b, target_b) = folders(&root, "b");
    let (source_c, target_c) = folders(&root, "c");
    let mut settings = Settings::default();
    let a = settings.add("A", &source_a, &target_a).unwrap();
    let b = settings.add("B", &source_b, &target_b).unwrap();
    let c = settings.add("C", &source_c, &target_c).unwrap();
    let revision = settings.sync_pairs[1].processing_revision;
    settings.rename(b, "Neu").unwrap();
    assert_eq!(settings.sync_pairs[1].processing_revision, revision);

    settings.select(b).unwrap();
    settings.remove(b).unwrap();
    assert_eq!(settings.selected_sync_pair_id, Some(c));
    settings.remove(c).unwrap();
    assert_eq!(settings.selected_sync_pair_id, Some(a));
    settings.remove(a).unwrap();
    assert_eq!(settings.selected_sync_pair_id, None);
}

#[test]
fn new_nonempty_target_is_refused_but_an_owned_target_can_be_readded() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    fs::write(target.join("unknown.txt"), "do not touch").unwrap();
    let mut settings = Settings::default();
    assert_eq!(
        settings.add("Book", &source, &target).unwrap_err().code,
        "target_not_empty"
    );
    assert_eq!(
        fs::read_to_string(target.join("unknown.txt")).unwrap(),
        "do not touch"
    );

    fs::remove_file(target.join("unknown.txt")).unwrap();
    let id = settings.add("Book", &source, &target).unwrap();
    fs::write(target.join("doc-0001.md"), "owned").unwrap();
    settings.remove(id).unwrap();
    assert_eq!(settings.add("Book again", &source, &target).unwrap(), id);
}

#[test]
fn initial_mapping_header_is_exact_and_corrupt_records_are_never_replaced() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "book");
    let mut settings = Settings::default();
    let id = settings.add("Book", &source, &target).unwrap();
    let mut mapping: Value =
        serde_json::from_slice(&fs::read(source.join("_document-mapping.json")).unwrap()).unwrap();
    // Volume-GUID and drive-letter Windows paths may identify the same directory.
    mapping["target_folder"] =
        json!(fs::canonicalize(mapping["target_folder"].as_str().unwrap()).unwrap());
    assert_eq!(
        mapping,
        json!({
            "schema_version": 1,
            "sync_pair_id": id,
            "target_folder": fs::canonicalize(&target).unwrap(),
            "next_document_number": 1,
            "entries": []
        })
    );

    let settings_path = root.path().join("settings.json");
    fs::write(&settings_path, b"not json").unwrap();
    assert_eq!(
        save_settings(&settings_path, &settings).unwrap_err().code,
        "invalid_settings"
    );
    assert_eq!(fs::read(&settings_path).unwrap(), b"not json");

    fs::write(source.join("_document-mapping.json"), b"not json").unwrap();
    settings.remove(id).unwrap();
    assert_eq!(
        settings.add("Again", &source, &target).unwrap_err().code,
        "invalid_mapping"
    );
    assert_eq!(
        fs::read(source.join("_document-mapping.json")).unwrap(),
        b"not json"
    );
}

#[test]
fn unknown_pair_ids_are_rejected() {
    let mut settings = Settings::default();
    let unknown = Uuid::new_v4();
    assert_eq!(settings.select(unknown).unwrap_err().code, "unknown_pair");
    assert_eq!(
        settings.rename(unknown, "Name").unwrap_err().code,
        "unknown_pair"
    );
    assert_eq!(settings.remove(unknown).unwrap_err().code, "unknown_pair");
}

fn fingerprint() -> ProcessingFingerprint {
    ProcessingFingerprint {
        engine_version: "test-1".into(),
        extraction_version: "1".into(),
        model_name: "de_core_news_lg".into(),
        model_version: "test-1".into(),
    }
}

#[test]
fn validated_config_is_local_to_one_pair_and_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let (a_source, a_target) = folders(&root, "a");
    let (b_source, b_target) = folders(&root, "b");
    let mut settings = Settings::default();
    let a = settings.add("A", &a_source, &a_target).unwrap();
    settings.add("B", &b_source, &b_target).unwrap();
    let b_bytes = serde_json::to_vec(&settings.sync_pairs[1]).unwrap();
    let previous = settings.sync_pairs[0].processing_revision;
    let mut config = ProcessingConfig::default();
    config.enabled_entities.clear();
    assert!(settings
        .apply_validated_config(a, config.clone(), fingerprint())
        .unwrap());
    assert_ne!(settings.sync_pairs[0].processing_revision, previous);
    assert_eq!(settings.sync_pairs[0].config, config);
    let bytes = serde_json::to_vec(&settings).unwrap();
    assert!(!settings
        .apply_validated_config(a, config, fingerprint())
        .unwrap());
    assert_eq!(serde_json::to_vec(&settings).unwrap(), bytes);
    assert_eq!(
        serde_json::to_vec(&settings.sync_pairs[1]).unwrap(),
        b_bytes
    );
    assert_eq!(
        settings
            .apply_validated_config(Uuid::new_v4(), ProcessingConfig::default(), fingerprint())
            .unwrap_err()
            .code,
        "unknown_pair"
    );
    assert_eq!(serde_json::to_vec(&settings).unwrap(), bytes);
}

#[test]
fn equivalent_entity_order_and_duplicate_words_preserve_saved_bytes() {
    let root = tempfile::tempdir().unwrap();
    let (source, target) = folders(&root, "a");
    let mut settings = Settings::default();
    let a = settings.add("A", &source, &target).unwrap();
    let rule_id = Uuid::new_v4();
    let mut config = ProcessingConfig::default();
    config.custom_rules.push(CustomRule::Words {
        id: rule_id,
        entity_type: EntityType::Custom,
        enabled: true,
        words: vec![" Anna ".into(), "anna".into(), " Anna ".into()],
    });
    // Simulate an already saved, non-canonical configuration.
    settings.sync_pairs[0].config = config.clone();
    settings.sync_pairs[0].processing_fingerprint = Some(fingerprint());
    let bytes = serde_json::to_vec(&settings).unwrap();
    config.enabled_entities.reverse();
    if let CustomRule::Words { words, .. } = &mut config.custom_rules[0] {
        words.pop();
    }
    assert!(!settings
        .apply_validated_config(a, config.clone(), fingerprint())
        .unwrap());
    assert_eq!(serde_json::to_vec(&settings).unwrap(), bytes);
    if let CustomRule::Words { words, .. } = &mut config.custom_rules[0] {
        words[0] = "Anna".into();
    }
    assert!(settings
        .apply_validated_config(a, config, fingerprint())
        .unwrap());
}

#[test]
fn fingerprint_fields_and_rule_identity_rotate_only_the_affected_pair_once() {
    let root = tempfile::tempdir().unwrap();
    let (a_source, a_target) = folders(&root, "a");
    let (b_source, b_target) = folders(&root, "b");
    let mut settings = Settings::default();
    let a = settings.add("A", &a_source, &a_target).unwrap();
    settings.add("B", &b_source, &b_target).unwrap();
    let b = settings.sync_pairs[1].clone();
    let mut config = ProcessingConfig::default();
    config.custom_rules.push(CustomRule::Regex {
        id: Uuid::new_v4(),
        entity_type: EntityType::Custom,
        enabled: true,
        pattern: "Anna".into(),
    });
    settings
        .apply_validated_config(a, config.clone(), fingerprint())
        .unwrap();
    let mut changed = fingerprint();
    for field in 0..4 {
        match field {
            0 => changed.engine_version = "test-2".into(),
            1 => changed.extraction_version = "2".into(),
            2 => {
                changed.model_name = "de_core_news_sm".into();
                config.model = changed.model_name.clone();
            }
            _ => changed.model_version = "test-2".into(),
        }
        let previous = settings.sync_pairs[0].processing_revision;
        assert!(settings
            .apply_validated_config(a, config.clone(), changed.clone())
            .unwrap());
        assert_ne!(settings.sync_pairs[0].processing_revision, previous);
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert!(!settings
            .apply_validated_config(a, config.clone(), changed.clone())
            .unwrap());
        assert_eq!(serde_json::to_vec(&settings).unwrap(), bytes);
        assert_eq!(settings.sync_pairs[1], b);
    }
    if let CustomRule::Regex { id, .. } = &mut config.custom_rules[0] {
        *id = Uuid::new_v4();
    }
    assert!(settings
        .apply_validated_config(a, config.clone(), changed.clone())
        .unwrap());
    config.include_positions = false;
    assert!(settings.apply_validated_config(a, config, changed).unwrap());
    assert_eq!(settings.sync_pairs[1], b);
}
