use std::path::PathBuf;

#[allow(dead_code)] // Task 4 consumes this cross-platform store fixture.
pub fn fixture_model_store(root: &std::path::Path) -> String {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/model-management.json"
    ))
    .unwrap();
    let mut descriptor = fixture["descriptor"].clone();
    let model = root.join("fixture-model");
    std::fs::create_dir_all(&model).unwrap();
    let files = [
        ("model.safetensors", b"synthetic weights".as_slice()),
        ("config.json", br#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":512,"id2label":{"0":"O","1":"B-DATE","2":"I-PERSON"}}"#.as_slice()),
        ("tokenizer.json", b"{}".as_slice()),
        ("tokenizer_config.json", br#"{"model_max_length":512}"#.as_slice()),
    ];
    descriptor["files"] = serde_json::Value::Array(files.iter().map(|(filename, bytes)| {
        use sha2::Digest;
        let hash = format!("{:x}", sha2::Sha256::digest(bytes));
        serde_json::json!({"filename": filename, "size": bytes.len(), "upstream_hash":{"algorithm":"sha256","value":hash}, "sha256":hash})
    }).collect());
    for (filename, bytes) in files {
        std::fs::write(model.join(filename), bytes).unwrap();
    }
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

#[allow(dead_code)]
pub fn manifest_dir() -> PathBuf {
    // Cross-built tests run with a native checkout/fixture mirror, not the build host's path.
    std::env::var_os("REDACTIO_TEST_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[allow(dead_code)]
pub fn fixture_model_name() -> String {
    serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../../../tests/fixtures/model-management.json"
    ))
    .unwrap()["descriptor"]["name"]
        .as_str()
        .unwrap()
        .to_owned()
}
#[allow(dead_code)]
pub fn fixture_model_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        fixture_model_store(root.path());
        root
    })
    .path()
    .into()
}
