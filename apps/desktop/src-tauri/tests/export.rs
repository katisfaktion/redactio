use redactio_lib::domain::export::validate_export_destination;
use std::fs;

#[test]
fn export_cannot_enter_another_pairs_source() {
    let root = tempfile::tempdir().unwrap();
    let other_source = root.path().join("other");
    let destination = other_source.join("export");
    let config = root.path().join("config");
    fs::create_dir_all(&destination).unwrap();
    fs::create_dir(&config).unwrap();
    let error = validate_export_destination(&destination, &[other_source], &config).unwrap_err();
    assert_eq!(error.code, "folder_overlap");
}

#[test]
fn every_saved_root_and_config_rejects_equal_child_and_ancestor_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let roots: Vec<_> = ["source", "target", "other-source", "other-target"]
        .map(|name| temp.path().join(name))
        .into();
    let config = temp.path().join("config");
    for root in roots.iter().chain([&config]) {
        fs::create_dir_all(root.join("export")).unwrap();
    }
    for root in roots.iter().chain([&config]) {
        for destination in [root.clone(), root.join("export"), temp.path().to_path_buf()] {
            assert_eq!(
                validate_export_destination(&destination, &roots, &config)
                    .unwrap_err()
                    .code,
                "folder_overlap"
            );
        }
    }
}

#[test]
fn empty_separate_directory_with_shared_name_prefix_is_accepted_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("source-export");
    let config = temp.path().join("config");
    for directory in [&source, &destination, &config] {
        fs::create_dir(directory).unwrap();
    }
    assert_eq!(
        validate_export_destination(&destination, &[source], &config).unwrap(),
        destination.canonicalize().unwrap()
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn unavailable_saved_root_or_config_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("export");
    let config = temp.path().join("config");
    let missing = temp.path().join("missing");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&config).unwrap();
    for (roots, config) in [(vec![missing.clone()], config), (vec![], missing)] {
        assert_eq!(
            validate_export_destination(&destination, &roots, &config)
                .unwrap_err()
                .code,
            "path_unavailable"
        );
    }
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn destination_must_be_an_existing_absolute_directory() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let file = temp.path().join("file");
    let missing = temp.path().join("missing");
    fs::create_dir(&config).unwrap();
    fs::write(&file, b"keep").unwrap();
    for (destination, code) in [
        (file.as_path(), "not_directory"),
        (missing.as_path(), "path_unavailable"),
        (std::path::Path::new("relative-export"), "invalid_path"),
    ] {
        assert_eq!(
            validate_export_destination(destination, &[], &config)
                .unwrap_err()
                .code,
            code
        );
    }
    assert_eq!(fs::read(file).unwrap(), b"keep");
    assert!(!missing.exists());
}

#[test]
fn nonempty_destination_preserves_existing_files_and_directories() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    for name in ["file-export", "directory-export"] {
        let destination = temp.path().join(name);
        fs::create_dir(&destination).unwrap();
        let existing = destination.join(".existing");
        if name == "file-export" {
            fs::write(&existing, b"keep").unwrap();
        } else {
            fs::create_dir(&existing).unwrap();
        }
        assert_eq!(
            validate_export_destination(&destination, &[], &config)
                .unwrap_err()
                .code,
            "export_not_empty"
        );
        assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
        if existing.is_file() {
            assert_eq!(fs::read(existing).unwrap(), b"keep");
        } else {
            assert!(existing.is_dir());
        }
    }
}

#[cfg(any(unix, windows))]
fn directory_alias(link: &std::path::Path, target: &std::path::Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let result = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "NTFS junction creation must succeed"
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn destination_links_and_linked_ancestors_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let real = temp.path().join("real");
    let alias = temp.path().join("alias");
    fs::create_dir(&config).unwrap();
    fs::create_dir_all(real.join("export")).unwrap();
    directory_alias(&alias, &real);
    for destination in [&alias, &alias.join("export")] {
        assert_eq!(
            validate_export_destination(destination, &[], &config)
                .unwrap_err()
                .code,
            "unsafe_object"
        );
    }
    assert_eq!(fs::read_dir(real.join("export")).unwrap().count(), 0);
    #[cfg(windows)]
    fs::remove_dir(alias).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn saved_root_and_config_aliases_cannot_hide_overlap() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let real = temp.path().join("real");
    let alias = temp.path().join("alias");
    let destination = real.join("export");
    fs::create_dir(&config).unwrap();
    fs::create_dir_all(&destination).unwrap();
    directory_alias(&alias, &real);
    for (roots, config) in [(vec![alias.clone()], config), (vec![], alias.clone())] {
        assert_eq!(
            validate_export_destination(&destination, &roots, &config)
                .unwrap_err()
                .code,
            "folder_overlap"
        );
    }
    #[cfg(windows)]
    fs::remove_dir(alias).unwrap();
}
