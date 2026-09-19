use redactio_lib::resources;
use std::{fs, path::Path};

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
    assert_eq!(models.len(), 2);
    assert!(models[0].compatible);
    assert!(!models[1].compatible);
    assert_eq!(models[0].name, "de_core_news_lg");
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
    fs::remove_file(root.join("webview2/msedgewebview2.exe")).unwrap();
    assert_eq!(
        resources::webview_directory(&executable).unwrap_err().code,
        "setup_incomplete"
    );
    fs::remove_file(root.join("models/de_core_news_lg/config.cfg")).unwrap();
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
