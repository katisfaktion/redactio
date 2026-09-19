use redactio_lib::domain::paths::validate_roots;
use std::fs;

#[test]
fn another_pairs_source_cannot_become_a_target() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let error = validate_roots(&a, &b, &[b.canonicalize().unwrap()]).unwrap_err();
    assert_eq!(error.code, "folder_overlap");
}

use redactio_lib::domain::paths::{create_target, validate_write};
use redactio_lib::domain::storage::{write_atomic, ValidatedWrite};

#[test]
fn all_pair_and_config_overlaps_are_rejected_by_components() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let ab = temp.path().join("ab");
    let child = a.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir(&ab).unwrap();
    for (source, target) in [(&a, &a), (&a, &child), (&child, &a)] {
        assert_eq!(
            validate_roots(source, target, &[]).unwrap_err().code,
            "folder_overlap"
        );
    }
    assert!(validate_roots(&a, &ab, &[]).is_ok());
    // The supplied roots include both sides of saved pairs and app configuration.
    for other in [&a, &ab, &child, temp.path()] {
        assert_eq!(
            validate_roots(&a, &ab, &[other.to_path_buf()])
                .unwrap_err()
                .code,
            "folder_overlap"
        );
    }
    let missing = temp.path().join("missing");
    assert!(validate_roots(&a, &ab, &[missing]).is_err());
    let file = temp.path().join("file");
    fs::write(&file, "unchanged").unwrap();
    assert!(validate_roots(&file, &ab, &[]).is_err());
    assert_eq!(fs::read(file).unwrap(), b"unchanged");
}

#[test]
fn only_explicit_target_creation_under_a_checked_parent_is_allowed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::create_dir(&source).unwrap();
    assert_eq!(
        create_target(&source, &target, &[], false)
            .unwrap_err()
            .code,
        "confirmation_required"
    );
    assert!(!target.exists());
    assert!(create_target(&source, &source.join("nested"), &[], true).is_err());
    assert!(!source.join("nested").exists());
    assert!(create_target(&source, &target, &[], true).is_ok());
    fs::write(target.join("owned-by-user"), "safe").unwrap();
    assert!(create_target(&source, &target, &[], true).is_err());
    assert_eq!(fs::read(target.join("owned-by-user")).unwrap(), b"safe");
}

#[test]
fn traversal_outside_root_and_non_regular_objects_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    for path in [
        root.join("../secret"),
        temp.path().join("secret"),
        root.clone(),
        root.join("missing/file"),
    ] {
        assert!(
            validate_write(&root, &path).is_err(),
            "unsafe route accepted"
        );
    }
    fs::create_dir(root.join("directory")).unwrap();
    assert!(validate_write(&root, &root.join("directory")).is_err());
    let checked = validate_write(&root, &root.join("state.json")).unwrap();
    fs::write(checked, b"safe").unwrap();
    assert_eq!(fs::read(root.join("state.json")).unwrap(), b"safe");
}

#[test]
fn private_atomic_create_replace_and_stale_object_preserve_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    write_atomic(&path, b"first").unwrap();
    write_atomic(&path, b"second").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"second");
    let checked = ValidatedWrite::new(temp.path(), &path).unwrap();
    fs::rename(&path, temp.path().join("old")).unwrap();
    fs::write(&path, b"external").unwrap();
    assert_eq!(
        checked.write_atomic(b"unsafe").unwrap_err().code,
        "path_changed"
    );
    assert_eq!(fs::read(&path).unwrap(), b"external");
    assert_eq!(fs::read(temp.path().join("old")).unwrap(), b"second");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(temp.path().join("old"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn absent_destination_cannot_overwrite_a_later_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    let checked = ValidatedWrite::new(temp.path(), &path).unwrap();
    fs::write(&path, b"external").unwrap();
    assert_eq!(
        checked.write_atomic(b"unsafe").unwrap_err().code,
        "path_changed"
    );
    assert_eq!(fs::read(&path).unwrap(), b"external");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn hardlinks_cannot_rewrite_source_documents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("document.docx");
    let target = temp.path().join("state.json");
    fs::write(&source, b"original").unwrap();
    fs::hard_link(&source, &target).unwrap();
    assert!(write_atomic(&target, b"replacement").is_err());
    assert_eq!(fs::read(source).unwrap(), b"original");
    assert_eq!(fs::read(target).unwrap(), b"original");
}

#[test]
fn errors_expose_only_safe_codes_and_retryability() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("CANARY_PERSON_NAME/private.json");
    let error = write_atomic(&path, b"secret").unwrap_err();
    let serialized = serde_json::to_value(&error).unwrap();
    assert_eq!(serialized.as_object().unwrap().len(), 2);
    assert!(serialized["code"].is_string());
    assert!(serialized["retryable"].is_boolean());
    assert!(!format!("{error:?} {error} {serialized}").contains("CANARY"));
}

#[cfg(unix)]
#[test]
fn symlinks_and_swapped_directories_cannot_redirect_writes() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    let victim = outside.join("state.json");
    fs::write(&victim, b"original").unwrap();
    symlink(&victim, root.join("state.json")).unwrap();
    assert!(validate_write(&root, &root.join("state.json")).is_err());
    assert!(write_atomic(&root.join("state.json"), b"unsafe").is_err());
    // An attacker-controlled fixed .tmp must be ignored and left untouched.
    symlink(&victim, root.join("other.json.tmp")).unwrap();
    write_atomic(&root.join("other.json"), b"safe").unwrap();
    assert!(fs::symlink_metadata(root.join("other.json.tmp"))
        .unwrap()
        .file_type()
        .is_symlink());
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let checked = ValidatedWrite::new(&root, &nested.join("state.json")).unwrap();
    fs::rename(&nested, root.join("old-nested")).unwrap();
    symlink(&outside, &nested).unwrap();
    assert_eq!(
        checked.write_atomic(b"unsafe").unwrap_err().code,
        "path_changed"
    );
    assert!(validate_write(&root, &nested.join("state.json")).is_err());
    assert_eq!(fs::read(victim).unwrap(), b"original");
    assert_eq!(fs::read_dir(root.join("old-nested")).unwrap().count(), 0);
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        ffi::OsString,
        fs::OpenOptions,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            fs::OpenOptionsExt,
        },
        path::Path,
        process::Command,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetShortPathNameW, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    fn junction(link: &Path, target: &Path) {
        let result = Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "NTFS junction creation must succeed for this test"
        );
    }

    #[test]
    fn case_and_junction_aliases_cannot_bypass_overlap() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Mixed Case Ä");
        fs::create_dir(&folder).unwrap();
        let upper = folder.to_str().unwrap().to_uppercase();
        assert_eq!(
            validate_roots(&folder, Path::new(&upper), &[])
                .unwrap_err()
                .code,
            "folder_overlap"
        );
        let alias = temp.path().join("junction");
        junction(&alias, &folder);
        assert_eq!(
            validate_roots(&folder, &alias, &[]).unwrap_err().code,
            "folder_overlap"
        );
        let child = folder.join("child");
        fs::create_dir(&child).unwrap();
        assert_eq!(
            validate_roots(&folder, &alias.join("child"), &[])
                .unwrap_err()
                .code,
            "folder_overlap"
        );
        fs::remove_dir(alias).unwrap();
    }

    #[test]
    fn short_path_alias_cannot_bypass_overlap() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Long directory name for short alias");
        fs::create_dir(&folder).unwrap();
        let input: Vec<_> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut output = vec![0u16; 32768];
        let length =
            unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
        assert!(length > 0 && (length as usize) < output.len());
        let short = std::path::PathBuf::from(OsString::from_wide(&output[..length as usize]));
        assert_ne!(short, folder, "8.3 names disabled: run this acceptance test on an NTFS volume with short names enabled");
        assert_eq!(
            validate_roots(&folder, &short, &[]).unwrap_err().code,
            "folder_overlap"
        );
    }

    #[test]
    fn a_temporary_file_symlink_is_not_used_or_deleted() {
        use std::os::windows::fs::symlink_file;
        let temp = tempfile::tempdir().unwrap();
        let victim = temp.path().join("document.docx");
        let target = temp.path().join("state.json");
        let trap = temp.path().join("state.json.tmp");
        fs::write(&victim, b"original").unwrap();
        symlink_file(&victim, &trap)
            .expect("Windows symlink acceptance test requires Developer Mode or symlink privilege");
        write_atomic(&target, b"safe").unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"original");
        assert!(fs::symlink_metadata(&trap)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"safe");
    }

    #[test]
    fn windows_special_names_and_streams_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        for name in [
            "file:stream",
            "name.",
            "name ",
            "NUL",
            "CON.txt",
            "COM1",
            "LPT¹",
        ] {
            assert!(validate_write(temp.path(), &temp.path().join(name)).is_err());
        }
    }

    #[test]
    fn junction_swapped_after_validation_cannot_redirect_writes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("state.json"), b"original").unwrap();
        let nested = root.join("nested");
        let checked = ValidatedWrite::new(&root, &nested.join("state.json")).unwrap();
        fs::rename(&nested, root.join("old-nested")).unwrap();
        junction(&nested, &outside);
        assert_eq!(
            checked.write_atomic(b"unsafe").unwrap_err().code,
            "path_changed"
        );
        assert!(validate_write(&root, &nested.join("state.json")).is_err());
        assert_eq!(fs::read(outside.join("state.json")).unwrap(), b"original");
        assert_eq!(fs::read_dir(root.join("old-nested")).unwrap().count(), 0);
        fs::remove_dir(nested).unwrap();
    }

    #[test]
    fn write_atomic_keeps_old_bytes_on_replace_failure() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("state.json");
        fs::write(&destination, b"original").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&destination)
            .unwrap();
        let error = write_atomic(&destination, b"replacement").unwrap_err();
        assert_eq!(error.code, "file_busy");
        assert!(error.retryable);
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(temp.path()).unwrap().count(),
            1,
            "operation-owned temporary and backup must be cleaned"
        );
        drop(held);
        write_atomic(&destination, b"replacement").unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"replacement");
    }
}

#[test]
fn previously_saved_roots_must_still_be_separate() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let previous = temp.path().join("previous");
    let nested = previous.join("nested");
    for dir in [&a, &b, &nested] {
        fs::create_dir_all(dir).unwrap();
    }
    assert_eq!(
        validate_roots(&a, &b, &[previous, nested])
            .unwrap_err()
            .code,
        "folder_overlap"
    );
}

#[cfg(unix)]
#[test]
fn unwritable_source_blocks_metadata_before_a_run() {
    use std::os::unix::fs::PermissionsExt;
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "permission regression must run as a standard user"
    );
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o500)).unwrap();
    let result = validate_roots(&source, &target, &[]);
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result.unwrap_err().code, "permission_denied");
    assert_eq!(fs::read_dir(source).unwrap().count(), 0);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}
