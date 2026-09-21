use redactio_lib::resources;
use std::{fs, path::Path};

fn package(root: &Path) {
    fs::create_dir_all(root.join("sidecar/_internal")).unwrap();
    fs::create_dir_all(root.join("webview2")).unwrap();
    fs::write(root.join("redactio.exe"), b"fixture").unwrap();
    fs::write(
        root.join("sidecar").join(if cfg!(windows) {
            "redactio-sidecar.exe"
        } else {
            "redactio-sidecar"
        }),
        b"fixture",
    )
    .unwrap();
    fs::write(root.join("webview2/msedgewebview2.exe"), b"fixture").unwrap();
}

#[test]
fn packaged_resources_allow_an_absent_model_store() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("Redactio Prüfung");
    package(&root);
    let executable = root.join("redactio.exe");

    let resolved = resources::resolve_packaged(&executable).unwrap();
    assert_eq!(resolved.model_root, root.join("models"));
    assert!(!resolved.model_root.exists());
    assert!(resources::list_models(&resolved.model_root)
        .unwrap()
        .is_empty());
    assert_eq!(
        resources::webview_directory(&executable).unwrap(),
        root.join("webview2")
    );
}

#[test]
fn packaged_resources_require_the_sidecar_and_webview_runtime() {
    let temporary = tempfile::tempdir().unwrap();
    package(temporary.path());
    let executable = temporary.path().join("redactio.exe");

    fs::remove_file(temporary.path().join("webview2/msedgewebview2.exe")).unwrap();
    assert_eq!(
        resources::webview_directory(&executable).unwrap_err().code,
        "setup_incomplete"
    );
    fs::remove_file(temporary.path().join("sidecar").join(if cfg!(windows) {
        "redactio-sidecar.exe"
    } else {
        "redactio-sidecar"
    }))
    .unwrap();
    let error = match resources::resolve_packaged(&executable) {
        Ok(_) => panic!("missing sidecar was accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "setup_incomplete");
}

#[cfg(unix)]
#[test]
fn packaged_resources_reject_redirected_nested_runtime_libraries() {
    let temporary = tempfile::tempdir().unwrap();
    package(temporary.path());
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        temporary.path().join("webview2/injected.dll"),
    )
    .unwrap();
    assert_eq!(
        resources::webview_directory(&temporary.path().join("redactio.exe"))
            .unwrap_err()
            .code,
        "setup_incomplete"
    );
}

#[cfg(unix)]
#[test]
fn packaged_resources_reject_a_linked_existing_model_store() {
    let temporary = tempfile::tempdir().unwrap();
    package(temporary.path());
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), temporary.path().join("models")).unwrap();
    assert!(resources::resolve_packaged(&temporary.path().join("redactio.exe")).is_err());
}

#[cfg(windows)]
#[test]
fn packaged_resources_reject_nested_runtime_junctions() {
    let temporary = tempfile::tempdir().unwrap();
    package(temporary.path());
    let outside = tempfile::tempdir().unwrap();
    let junction = temporary.path().join("webview2/redirected");
    let status = std::process::Command::new("cmd.exe")
        .current_dir(temporary.path())
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let result = resources::webview_directory(&temporary.path().join("redactio.exe"));
    fs::remove_dir(junction).unwrap();
    assert_eq!(result.unwrap_err().code, "setup_incomplete");
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
