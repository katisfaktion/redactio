use redactio_lib::resources;
use std::{fs, path::Path};

const BIOMEDBERT: &str = "OpenMed-PII-German-BiomedBERT-Large-340M-v1";
const BIOMEDBERT_VERSION: &str = "ce797d58600cc20bba9a2500dafc0b7f5c3270c1";
const HUGGINGLIL: &str = "pii-sensitive-ner-german";
const HUGGINGLIL_VERSION: &str = "6af88facbb75da7be737da55d2c411c7ce79e5a1";

fn biomedbert(root: &Path) {
    let model = root.join("biomedbert-de");
    fs::create_dir(&model).unwrap();
    for name in [
        "model.safetensors",
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "vocab.txt",
    ] {
        fs::write(model.join(name), b"fixture").unwrap();
    }
    fs::write(
        model.join("config.json"),
        r#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":512,"id2label":{"0":"O","1":"B-FIRSTNAME","2":"I-FIRSTNAME","3":"B-LASTNAME","4":"B-ZIPCODE"}}"#,
    )
    .unwrap();
    fs::write(
        model.join("redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": BIOMEDBERT,
            "version": BIOMEDBERT_VERSION,
            "repository": format!("OpenMed/{BIOMEDBERT}"),
        }))
        .unwrap(),
    )
    .unwrap();
}

fn hugginglil(root: &Path) {
    let model = root.join(HUGGINGLIL);
    fs::create_dir(&model).unwrap();
    for name in [
        "model.safetensors",
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "added_tokens.json",
        "spm.model",
    ] {
        fs::write(model.join(name), b"fixture").unwrap();
    }
    fs::write(
        model.join("config.json"),
        r#"{"architectures":["DebertaV2ForTokenClassification"],"model_type":"deberta-v2","max_position_embeddings":512,"id2label":{"0":"I-ACCOUNTNUM","1":"I-BUILDINGNUM","2":"I-CITY","3":"I-CREDITCARDNUMBER","4":"I-DATEOFBIRTH","5":"I-DRIVERLICENSENUM","6":"I-EMAIL","7":"I-GIVENNAME","8":"I-IDCARDNUM","9":"I-PASSWORD","10":"I-SOCIALNUM","11":"I-STREET","12":"I-SURNAME","13":"I-TAXNUM","14":"I-TELEPHONENUM","15":"I-USERNAME","16":"I-ZIPCODE","17":"O","18":"I-REL","19":"I-ETHN","20":"I-SOR"}}"#,
    )
    .unwrap();
    fs::write(
        model.join("redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": HUGGINGLIL,
            "version": HUGGINGLIL_VERSION,
            "repository": format!("HuggingLil/{HUGGINGLIL}"),
        }))
        .unwrap(),
    )
    .unwrap();
}

fn manifest_models() -> serde_json::Value {
    serde_json::json!([
        {"name": HUGGINGLIL, "version": HUGGINGLIL_VERSION, "path": HUGGINGLIL},
        {"name": BIOMEDBERT, "version": BIOMEDBERT_VERSION, "path": "biomedbert-de"},
    ])
}

fn package(root: &Path) {
    fs::create_dir_all(root.join("sidecar/_internal")).unwrap();
    fs::create_dir_all(root.join("models/de_core_news_lg")).unwrap();
    fs::create_dir_all(root.join("webview2")).unwrap();
    for name in [
        "redactio.exe",
        "models/manifest.json",
        "models/de_core_news_lg/config.cfg",
        "models/de_core_news_lg/meta.json",
        "webview2/msedgewebview2.exe",
    ] {
        fs::write(root.join(name), b"fixture").unwrap();
    }
    fs::write(
        root.join("sidecar").join(if cfg!(windows) {
            "redactio-sidecar.exe"
        } else {
            "redactio-sidecar"
        }),
        b"fixture",
    )
    .unwrap();
    biomedbert(&root.join("models"));
    hugginglil(&root.join("models"));
    fs::write(
        root.join("models/manifest.json"),
        serde_json::to_vec(&serde_json::json!({"models": manifest_models()})).unwrap(),
    )
    .unwrap();
}

#[test]
fn model_listing_supports_both_pinned_native_packages_in_preferred_order() {
    let root = tempfile::tempdir().unwrap();
    biomedbert(root.path());
    hugginglil(root.path());
    fs::write(
        root.path().join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({"models": manifest_models()})).unwrap(),
    )
    .unwrap();

    let listed = resources::list_models(root.path()).unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|model| model.name.as_str())
            .collect::<Vec<_>>(),
        [BIOMEDBERT, HUGGINGLIL]
    );
    assert!(listed.iter().all(|model| model.compatible));
    assert_eq!(listed[1].entity_types.len(), 20);
    assert!(listed[1]
        .entity_types
        .iter()
        .any(|label| label.as_str() == "SOR"));

    let config = root.path().join(HUGGINGLIL).join("config.json");
    let valid_config = fs::read(&config).unwrap();
    fs::write(&config, r#"{"architectures":["BertForTokenClassification"],"model_type":"deberta-v2","max_position_embeddings":512,"id2label":{"0":"I-ACCOUNTNUM","1":"O"}}"#).unwrap();
    assert!(!resources::list_models(root.path()).unwrap()[1].compatible);
    fs::write(&config, r#"{"architectures":["DebertaV2ForTokenClassification"],"model_type":"deberta-v2","max_position_embeddings":512,"id2label":{"0":"I-FUTURE_LABEL","1":"O"}}"#).unwrap();
    let listed = resources::list_models(root.path()).unwrap();
    assert!(listed[1].compatible);
    assert_eq!(listed[1].entity_types[0].as_str(), "FUTURE_LABEL");
    fs::write(&config, valid_config).unwrap();
    fs::write(
        root.path().join(HUGGINGLIL).join("redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": HUGGINGLIL,
            "version": "different-revision",
            "repository": format!("HuggingLil/{HUGGINGLIL}"),
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!resources::list_models(root.path()).unwrap()[1].compatible);
}

#[test]
fn model_listing_uses_only_local_manifest_packages_and_rejects_unsafe_entries() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lg")).unwrap();
    fs::write(root.path().join("lg/config.cfg"), "synthetic").unwrap();
    fs::write(
        root.path().join("lg/meta.json"),
        r#"{"lang":"de","name":"core_news_lg","version":"3.8.0","spacy_version":">=3.8.0,<3.9.0"}"#,
    )
    .unwrap();
    let entry = serde_json::json!({"name":"de_core_news_lg","version":"3.8.0","path":"lg"});
    let missing = serde_json::json!({"name":"de_core_news_sm","version":"3.8.0","path":"sm"});
    let manifest = root.path().join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&serde_json::json!({"models":[entry,missing]})).unwrap(),
    )
    .unwrap();
    let models = resources::list_models(root.path()).unwrap();
    assert!(models.is_empty());
    for models in [
        serde_json::json!([entry, entry]),
        serde_json::json!([{ "name":"de_core_news_lg","version":"3.8.0","path":"../outside" }]),
    ] {
        fs::write(
            &manifest,
            serde_json::to_vec(&serde_json::json!({"models":models})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            resources::list_models(root.path()).unwrap_err().code,
            "invalid_model_manifest"
        );
    }
}

#[test]
fn model_listing_accepts_only_the_complete_pinned_biomedbert_package() {
    let root = tempfile::tempdir().unwrap();
    biomedbert(root.path());
    fs::write(
        root.path().join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({"models":[{
            "name": BIOMEDBERT,
            "version": BIOMEDBERT_VERSION,
            "path": "biomedbert-de",
        }]}))
        .unwrap(),
    )
    .unwrap();

    let listed = || resources::list_models(root.path()).unwrap()[0].compatible;
    assert!(listed());
    assert_eq!(
        resources::list_models(root.path()).unwrap()[0].entity_types,
        ["FIRSTNAME", "LASTNAME", "ZIPCODE"]
            .into_iter()
            .map(|label| redactio_lib::domain::settings::EntityType::parse(label.into()).unwrap())
            .collect::<Vec<_>>()
    );
    let valid_config = fs::read(root.path().join("biomedbert-de/config.json")).unwrap();
    fs::write(
        root.path().join("biomedbert-de/config.json"),
        r#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":512}"#,
    )
    .unwrap();
    assert!(
        !listed(),
        "a BiomedBERT package without native label metadata is incompatible"
    );
    fs::write(root.path().join("biomedbert-de/config.json"), valid_config).unwrap();

    fs::remove_file(root.path().join("biomedbert-de/model.safetensors")).unwrap();
    assert!(!listed());
    fs::write(
        root.path().join("biomedbert-de/model.safetensors"),
        b"fixture",
    )
    .unwrap();

    fs::remove_file(root.path().join("biomedbert-de/redactio-model.json")).unwrap();
    assert!(!listed());
    fs::write(
        root.path().join("biomedbert-de/redactio-model.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": BIOMEDBERT,
            "version": "different-revision",
            "repository": format!("OpenMed/{BIOMEDBERT}"),
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!listed());
}

#[test]
fn packaged_resources_are_absolute_and_missing_runtime_or_model_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Redactio Prüfung");
    package(&root);
    let executable = root.join("redactio.exe");
    let resolved = resources::resolve_packaged(&executable).unwrap();
    assert_eq!(resolved.model_root, root.join("models"));
    assert!(resolved.sidecar_executable.is_absolute());
    assert_eq!(
        resources::webview_directory(&executable).unwrap(),
        root.join("webview2")
    );
    fs::remove_file(root.join("models/pii-sensitive-ner-german/config.json")).unwrap();
    assert!(resources::resolve_packaged(&executable).is_ok());
    fs::remove_file(root.join("webview2/msedgewebview2.exe")).unwrap();
    assert_eq!(
        resources::webview_directory(&executable).unwrap_err().code,
        "setup_incomplete"
    );
    fs::remove_file(root.join("models/biomedbert-de/config.json")).unwrap();
    assert!(resources::resolve_packaged(&executable).is_err());
    assert!(resources::resolve_packaged(Path::new("redactio.exe")).is_err());
}

#[cfg(unix)]
#[test]
fn packaged_resources_reject_redirected_nested_runtime_libraries() {
    let temp = tempfile::tempdir().unwrap();
    package(temp.path());
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("webview2/injected.dll")).unwrap();
    assert_eq!(
        resources::webview_directory(&temp.path().join("redactio.exe"))
            .unwrap_err()
            .code,
        "setup_incomplete"
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn release_ignores_development_resource_overrides() {
    let executable = std::env::current_exe().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::env::set_var("REDACTIO_SIDECAR_EXECUTABLE", executable);
    std::env::set_var("REDACTIO_MODEL_DIR", root.path());
    assert!(resources::resolve().is_err());
}

#[cfg(windows)]
#[test]
fn packaged_resources_reject_nested_runtime_junctions() {
    let temp = tempfile::tempdir().unwrap();
    package(temp.path());
    let outside = tempfile::tempdir().unwrap();
    let junction = temp.path().join("webview2").join("redirected");
    let status = std::process::Command::new("cmd.exe")
        .current_dir(temp.path())
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let result = resources::webview_directory(&temp.path().join("redactio.exe"));
    fs::remove_dir(junction).unwrap();
    assert_eq!(result.unwrap_err().code, "setup_incomplete");
}
