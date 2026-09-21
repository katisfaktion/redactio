use std::path::PathBuf;

#[allow(dead_code)] // Task 4 consumes this cross-platform store fixture.
pub fn fixture_model_store(root: &std::path::Path) -> String {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/model-management.json"
    ))
    .unwrap();
    let descriptor = fixture["descriptor"].clone();
    let model = root.join("fixture-model");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("model.safetensors"), vec![0_u8; 1024]).unwrap();
    std::fs::write(
        model.join("config.json"),
        r#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":512,"id2label":{"0":"O","1":"B-DATE","2":"I-PERSON"}}"#,
    )
    .unwrap();
    std::fs::write(model.join("tokenizer.json"), "{}").unwrap();
    std::fs::write(model.join("tokenizer_config.json"), "{}").unwrap();
    std::fs::write(
        model.join("redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": descriptor["name"],
            "version": descriptor["version"],
            "repository": descriptor["repository"],
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "models": [{"descriptor": descriptor, "path": "fixture-model", "state": "ready"}],
            "legacy_unavailable": [],
        }))
        .unwrap(),
    )
    .unwrap();
    fixture["descriptor"]["name"].as_str().unwrap().to_owned()
}

pub fn manifest_dir() -> PathBuf {
    // Cross-built tests run with a native checkout/fixture mirror, not the build host's path.
    std::env::var_os("REDACTIO_TEST_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}
