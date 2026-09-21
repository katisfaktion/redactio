mod common;
use redactio_lib::model_store;
use std::fs;

#[test]
fn missing_store_keeps_the_catalog_available_without_writing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("models");

    assert!(model_store::list_models(&root).unwrap().is_empty());
    assert_eq!(model_store::list_managed(&root).unwrap().len(), 2);
    assert!(!root.exists());
}

fn descriptor() -> serde_json::Value {
    serde_json::json!({
        "name": "hf:acme/medical-ner@0123456789abcdef0123456789abcdef01234567",
        "version": "0123456789abcdef0123456789abcdef01234567",
        "repository": "acme/medical-ner",
        "title": "Synthetic medical NER",
        "license": "Apache-2.0",
        "model_type": "bert",
        "architecture": "BertForTokenClassification",
        "entity_types": ["DATE", "PERSON"],
        "window_tokens": 1024,
        "stride_tokens": 256,
        "special_tokens": 2,
        "files": [{
            "filename": "model.safetensors",
            "size": 3,
            "upstream_hash": {"algorithm": "sha256", "value": "a".repeat(64)},
            "sha256": "b".repeat(64),
        }],
    })
}

#[test]
fn strict_dtos_reject_unknown_labels_paths_and_future_registries() {
    let descriptor = descriptor();
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(descriptor.clone()).is_ok());
    let mut unknown_field = descriptor.clone();
    unknown_field["private_path"] = serde_json::json!("/model");
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(unknown_field).is_err());
    assert!(
        serde_json::from_value::<model_store::ModelRegistry>(serde_json::json!({
            "schema_version": 3, "models": [], "legacy_unavailable": [],
        }))
        .is_err()
    );
    let mut generic_label = descriptor.clone();
    generic_label["entity_types"] = serde_json::json!(["LABEL_0"]);
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(generic_label).is_ok());
    assert!(
        serde_json::from_value::<model_store::ModelRegistry>(serde_json::json!({
            "schema_version": 2, "models": [],
            "legacy_unavailable": [{"name": "legacy", "version": "v1", "path": "../outside"}],
        }))
        .is_err()
    );
    let mut missing_nullable = descriptor;
    missing_nullable.as_object_mut().unwrap().remove("license");
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(missing_nullable).is_err());
}

#[test]
fn shared_management_fixture_parses_at_every_rust_boundary() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/model-management.json"
    ))
    .unwrap();
    assert!(serde_json::from_value::<model_store::Artifact>(fixture["artifact"].clone()).is_ok());
    assert!(
        serde_json::from_value::<model_store::ModelDescriptor>(fixture["descriptor"].clone())
            .is_ok()
    );
    assert!(
        serde_json::from_value::<model_store::CatalogEntry>(fixture["catalog_entry"].clone())
            .is_ok()
    );
    assert!(
        serde_json::from_value::<model_store::ModelCatalog>(fixture["catalog"].clone()).is_ok()
    );
    assert!(
        serde_json::from_value::<model_store::ModelRegistry>(fixture["registry"].clone()).is_ok()
    );
    assert!(
        serde_json::from_value::<model_store::ManagedModel>(fixture["managed_model"].clone())
            .is_ok()
    );
    for source in fixture["sources"].as_array().unwrap() {
        assert!(serde_json::from_value::<model_store::ModelSource>(source.clone()).is_ok());
    }
    assert!(
        serde_json::from_value::<model_store::CheckedModel>(fixture["checked_model"].clone())
            .is_ok()
    );
    assert!(serde_json::from_value::<model_store::ModelJob>(fixture["job"].clone()).is_ok());
}

#[test]
fn imported_selection_uses_repository_and_revision() {
    let revision = "a".repeat(40);
    assert_eq!(
        model_store::selection_name("one/model", &revision),
        format!("hf:one/model@{revision}")
    );
    assert_ne!(
        model_store::selection_name("one/model", &revision),
        model_store::selection_name("two/model", &revision)
    );
}

#[test]
fn legacy_manifest_projects_known_metadata_without_rewriting_or_weight_hashing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let catalog = model_store::catalog_models().unwrap();
    let entry = &catalog[0];
    let model = root.join(&entry.directory);
    fs::create_dir(&model).unwrap();
    for file in &entry.descriptor.files {
        fs::write(model.join(&file.filename), b"fixture").unwrap();
    }
    fs::write(
        model.join("config.json"),
        r#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":512,"id2label":{"0":"O","1":"B-DATE"}}"#,
    )
    .unwrap();
    fs::write(
        model.join("tokenizer_config.json"),
        r#"{"model_max_length":512}"#,
    )
    .unwrap();
    fs::write(
        model.join("redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": entry.descriptor.name,
            "version": entry.descriptor.version,
            "repository": entry.descriptor.repository,
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "models": [
                {"name": entry.descriptor.name, "version": entry.descriptor.version, "path": entry.directory},
                {"name": "retired", "version": "v1", "path": "retired/model"},
            ],
        }))
        .unwrap(),
    )
    .unwrap();

    let registry = model_store::read_registry(root).unwrap();
    assert_eq!(registry.schema_version, 2);
    assert_eq!(registry.models.len(), 1);
    assert_eq!(registry.legacy_unavailable.len(), 1);
    assert!(model_store::list_models(root).unwrap()[0].compatible);
    assert_eq!(
        model_store::list_managed(root).unwrap()[0].state,
        model_store::ManagedState::Ready
    );
    assert!(root.join("manifest.json").exists());
}

#[test]
fn ready_registry_record_is_discovered_without_loading_weights() {
    let temporary = tempfile::tempdir().unwrap();
    let output = std::env::var_os("REDACTIO_FIXTURE_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_owned());
    fs::create_dir_all(&output).unwrap();
    let name = common::fixture_model_store(&output);
    let models = model_store::list_models(&output).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, name);
    assert_eq!(models[0].entity_types.len(), 2);
}

#[test]
fn discovery_requires_core_metadata_and_agrees_on_invalid_state() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let model = root.join("fixture-model");
    fs::create_dir(&model).unwrap();
    fs::write(model.join("model.safetensors"), b"abc").unwrap();
    fs::write(
        model.join("config.json"),
        r#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":1024,"id2label":{"0":"O","1":"B-DATE","2":"I-PERSON"}}"#,
    )
    .unwrap();
    let descriptor = descriptor();
    fs::write(
        model.join("redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({"name": descriptor["name"], "version": descriptor["version"], "repository": descriptor["repository"]})).unwrap(),
    ).unwrap();
    fs::write(root.join("manifest.json"), serde_json::to_vec(&serde_json::json!({"schema_version":2,"models":[{"descriptor":descriptor,"path":"fixture-model","state":"ready"}],"legacy_unavailable":[]})).unwrap()).unwrap();

    assert!(!model_store::list_models(root).unwrap()[0].compatible);
    assert_eq!(
        model_store::list_managed(root).unwrap()[2].state,
        model_store::ManagedState::Invalid
    );
}

#[test]
fn empty_titles_are_transport_values() {
    let mut value = descriptor();
    value["title"] = serde_json::json!("");
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(value).is_ok());
}

#[test]
fn opaque_names_count_unicode_scalar_values() {
    let mut value = descriptor();
    value["name"] = serde_json::json!("😀".repeat(257));
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(value.clone()).is_ok());
    value["name"] = serde_json::json!("😀".repeat(513));
    assert!(serde_json::from_value::<model_store::ModelDescriptor>(value).is_err());
}

#[cfg(unix)]
#[test]
fn linked_store_or_manifest_is_an_invalid_store_not_an_empty_one() {
    let temporary = tempfile::tempdir().unwrap();
    let linked = temporary.path().join("linked-models");
    std::os::unix::fs::symlink(temporary.path().join("missing"), &linked).unwrap();
    assert_eq!(
        model_store::read_registry(&linked).unwrap_err().code,
        "invalid_model_manifest"
    );

    let root = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(
        temporary.path().join("missing-manifest"),
        root.path().join("manifest.json"),
    )
    .unwrap();
    assert_eq!(
        model_store::read_registry(root.path()).unwrap_err().code,
        "invalid_model_manifest"
    );
}

#[test]
fn v2_discovery_uses_effective_tokenizer_window_and_canonical_identity() {
    let temporary = tempfile::tempdir().unwrap();
    common::fixture_model_store(temporary.path());
    let root = temporary.path();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let config = r#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":1024,"id2label":{"0":"O","1":"B-DATE","2":"I-PERSON"}}"#;
    fs::write(root.join("fixture-model/config.json"), config).unwrap();
    let files = manifest["models"][0]["descriptor"]["files"]
        .as_array_mut()
        .unwrap();
    files
        .iter_mut()
        .find(|file| file["filename"] == "config.json")
        .unwrap()["size"] = serde_json::json!(config.len());
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(model_store::list_models(root).unwrap()[0].compatible);

    manifest["models"][0]["descriptor"]["name"] = serde_json::json!("pii-sensitive-ner-german");
    fs::write(root.join("fixture-model/redactio-model.json"), serde_json::to_vec(&serde_json::json!({"name":"pii-sensitive-ner-german","version":manifest["models"][0]["descriptor"]["version"],"repository":manifest["models"][0]["descriptor"]["repository"]})).unwrap()).unwrap();
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(!model_store::list_models(root).unwrap()[0].compatible);
    assert_eq!(
        model_store::list_managed(root).unwrap()[2].state,
        model_store::ManagedState::Invalid
    );
}
