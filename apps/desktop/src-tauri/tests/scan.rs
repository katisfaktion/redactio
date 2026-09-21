use redactio_lib::domain::scan::scan_source;
use std::fs;

#[test]
fn scan_filters_metadata_lockfiles_and_hidden_children() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::create_dir(root.path().join(".hidden")).unwrap();
    for name in [
        "a.DOCX",
        "nested/b.docx",
        "~$locked.docx",
        ".hidden.docx",
        ".hidden/child.docx",
        "_document-mapping.json",
        "note.txt",
    ] {
        fs::write(root.path().join(name), b"synthetic").unwrap();
    }

    let report = scan_source(root.path()).unwrap();
    let names: Vec<_> = report
        .files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();

    assert_eq!(names, vec!["a.DOCX", "nested/b.docx"]);
    assert!(report.errors.is_empty());
}

#[test]
fn scan_returns_stable_paths_hashes_and_new_state() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("z")).unwrap();
    fs::write(root.path().join("z/document.docx"), b"synthetic").unwrap();
    fs::write(root.path().join("first.docx"), b"synthetic").unwrap();

    let report = scan_source(root.path()).unwrap();

    assert_eq!(report.files[0].relative_path, "first.docx");
    assert_eq!(report.files[1].relative_path, "z/document.docx");
    assert_eq!(report.files[0].size_bytes, Some(9));
    assert_eq!(
        report.files[0].source_hash_sha256.as_deref(),
        Some("b3cc0475bb78a5026098858e9889acf666d31062d513d303314eca31d36e72f2")
    );
    assert_eq!(format!("{:?}", report.files[0].state), "New");
    assert!(report.files[0].mtime.as_ref().unwrap().ends_with('Z'));
}

#[test]
fn a_missing_or_renamed_source_is_a_run_level_error() {
    let parent = tempfile::tempdir().unwrap();
    let source = parent.path().join("source");
    let renamed = parent.path().join("renamed");
    fs::create_dir(&source).unwrap();
    fs::rename(&source, &renamed).unwrap();

    let error = scan_source(&source).unwrap_err();

    assert_eq!(error.code, "path_unavailable");
    assert!(scan_source(&renamed).unwrap().files.is_empty());
}

#[cfg(unix)]
#[test]
fn scan_does_not_traverse_symlinks() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("outside.docx"), b"outside").unwrap();
    fs::write(root.path().join("inside.docx"), b"inside").unwrap();
    symlink(outside.path(), root.path().join("linked-directory")).unwrap();
    symlink(
        outside.path().join("outside.docx"),
        root.path().join("linked-file.docx"),
    )
    .unwrap();

    let report = scan_source(root.path()).unwrap();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative_path, "inside.docx");
    assert!(report.errors.is_empty());
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_reported_without_discarding_readable_peers() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let unreadable = root.path().join("blocked.docx");
    fs::write(&unreadable, b"blocked").unwrap();
    fs::write(root.path().join("readable.docx"), b"readable").unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

    let report = scan_source(root.path()).unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative_path, "readable.docx");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].relative_path, "blocked.docx");
    assert_eq!(report.errors[0].code, "permission_denied");
}

#[cfg(unix)]
#[test]
fn a_non_unicode_document_name_is_a_file_error() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let root = tempfile::tempdir().unwrap();
    let name = OsString::from_vec(vec![0xff, b'.', b'd', b'o', b'c', b'x']);
    fs::write(root.path().join(name), b"synthetic").unwrap();
    fs::write(root.path().join("readable.docx"), b"readable").unwrap();

    let report = scan_source(root.path()).unwrap();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative_path, "readable.docx");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "invalid_path");
}

#[cfg(windows)]
#[test]
fn scan_filters_windows_hidden_files_and_junctions() {
    use std::process::Command;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let hidden = root.path().join("hidden.docx");
    fs::write(&hidden, b"hidden").unwrap();
    fs::write(root.path().join("visible.docx"), b"visible").unwrap();
    fs::write(outside.path().join("outside.docx"), b"outside").unwrap();
    assert!(Command::new("attrib")
        .arg("+H")
        .arg(&hidden)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.path().join("junction"))
        .arg(outside.path())
        .status()
        .unwrap()
        .success());

    let report = scan_source(root.path()).unwrap();
    let _ = Command::new("attrib").arg("-H").arg(&hidden).status();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative_path, "visible.docx");
    assert!(report.errors.is_empty());
}

#[cfg(windows)]
#[test]
fn an_unreadable_file_is_reported_from_a_child_process() {
    use std::process::Command;

    let root = tempfile::tempdir().unwrap();
    let blocked = root.path().join("blocked.docx");
    fs::write(&blocked, b"blocked").unwrap();
    fs::write(root.path().join("readable.docx"), b"readable").unwrap();
    assert!(Command::new("icacls")
        .arg(&blocked)
        .args(["/inheritance:r", "/deny", "*S-1-1-0:(R)"])
        .status()
        .unwrap()
        .success());

    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "scan_windows_permission_child", "--nocapture"])
        .env("REDACTIO_SCAN_PERMISSION_ROOT", root.path())
        .output()
        .unwrap();
    let _ = Command::new("icacls")
        .arg(&blocked)
        .args(["/remove:d", "*S-1-1-0", "/inheritance:e"])
        .status();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
#[test]
fn scan_windows_permission_child() {
    let Some(root) = std::env::var_os("REDACTIO_SCAN_PERMISSION_ROOT") else {
        eprintln!("skipped: only the parent test supplies an ACL-restricted source");
        return;
    };

    let report = scan_source(std::path::Path::new(&root)).unwrap();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative_path, "readable.docx");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].relative_path, "blocked.docx");
    assert_eq!(report.errors[0].code, "permission_denied");
}
