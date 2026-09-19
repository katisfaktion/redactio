use super::storage::open_validated_read;
use crate::error::AppError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub files: Vec<ScannedFile>,
    pub errors: Vec<ScanFailure>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ScannedFile {
    pub relative_path: String,
    pub size_bytes: u64,
    pub mtime: String,
    pub source_hash_sha256: String,
    pub state: ScanState,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanState {
    New,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ScanFailure {
    pub relative_path: String,
    pub code: String,
}

pub fn scan_source(source: &Path) -> Result<ScanReport, AppError> {
    scan_source_with(source, |_| {})
}

fn scan_source_with(
    source: &Path,
    mut before_open: impl FnMut(&Path),
) -> Result<ScanReport, AppError> {
    let root_metadata = fs::metadata(source)?;
    if !root_metadata.is_dir() {
        return Err(AppError::new("not_directory"));
    }
    fs::read_dir(source)?;

    let mut report = ScanReport {
        files: Vec::new(),
        errors: Vec::new(),
    };
    let entries = WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_ignored(entry));

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(path) = error
                    .path()
                    .and_then(|path| display_relative_path(source, path))
                {
                    report.errors.push(ScanFailure {
                        relative_path: path,
                        code: safe_io_code(error.io_error().map(io::Error::kind)).into(),
                    });
                    continue;
                }
                return Err(AppError::new(safe_io_code(
                    error.io_error().map(io::Error::kind),
                )));
            }
        };
        if entry.depth() == 0 || is_ignored(&entry) || !entry.file_type().is_file() {
            continue;
        }
        let Some(display_path) = display_relative_path(source, entry.path()) else {
            continue;
        };
        if !entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
        {
            continue;
        }
        let Some(relative_path) = relative_path(source, entry.path()) else {
            report.errors.push(ScanFailure {
                relative_path: display_path,
                code: "invalid_path".into(),
            });
            continue;
        };
        before_open(entry.path());
        match scan_file(source, entry.path(), relative_path.clone()) {
            Ok(file) => report.files.push(file),
            Err(code) => report.errors.push(ScanFailure {
                relative_path,
                code,
            }),
        }
    }

    report
        .files
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    report
        .errors
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(report)
}

fn scan_file(root: &Path, path: &Path, relative_path: String) -> Result<ScannedFile, String> {
    let mut file = open_validated_read(root, path).map_err(|error| error.code)?;
    let metadata = file.metadata().map_err(io_code)?;
    let mtime = metadata
        .modified()
        .map(OffsetDateTime::from)
        .and_then(|value| {
            value
                .format(&Rfc3339)
                .map_err(|_| io::Error::other("invalid timestamp"))
        })
        .map_err(io_code)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_code)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(ScannedFile {
        relative_path,
        size_bytes: metadata.len(),
        mtime,
        source_hash_sha256: format!("{:x}", hasher.finalize()),
        state: ScanState::New,
    })
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_str()?.to_owned()),
            _ => return None,
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

fn display_relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

fn is_ignored(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    name.starts_with('.')
        || name.starts_with("~$")
        || name == "_document-mapping.json"
        || entry.file_type().is_symlink()
        || has_ignored_windows_attributes(entry.path())
}

#[cfg(windows)]
fn has_ignored_windows_attributes(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    fs::symlink_metadata(path).is_ok_and(|metadata| {
        let attributes = metadata.file_attributes();
        attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_REPARSE_POINT) != 0
    })
}

#[cfg(not(windows))]
fn has_ignored_windows_attributes(_path: &Path) -> bool {
    false
}

fn io_code(error: io::Error) -> String {
    safe_io_code(Some(error.kind())).into()
}

fn safe_io_code(kind: Option<io::ErrorKind>) -> &'static str {
    match kind {
        Some(io::ErrorKind::NotFound) => "path_unavailable",
        Some(io::ErrorKind::PermissionDenied) => "permission_denied",
        _ => "storage_io",
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_discovered_file_replaced_by_a_symlink_is_a_file_error() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = root.path().join("victim.docx");
        let outside_file = outside.path().join("outside.docx");
        fs::write(&victim, b"inside").unwrap();
        fs::write(root.path().join("readable.docx"), b"readable").unwrap();
        fs::write(&outside_file, b"outside").unwrap();

        let report = scan_source_with(root.path(), |path| {
            if path == victim {
                fs::remove_file(path).unwrap();
                symlink(&outside_file, path).unwrap();
            }
        })
        .unwrap();

        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].relative_path, "readable.docx");
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].relative_path, "victim.docx");
    }

    #[test]
    fn a_discovered_parent_replaced_by_a_symlink_is_a_file_error() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        let moved = root.path().join("moved");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("victim.docx"), b"inside").unwrap();
        fs::write(root.path().join("readable.docx"), b"readable").unwrap();
        fs::write(outside.path().join("victim.docx"), b"outside").unwrap();

        let report = scan_source_with(root.path(), |path| {
            if path == nested.join("victim.docx") {
                fs::rename(&nested, &moved).unwrap();
                symlink(outside.path(), &nested).unwrap();
            }
        })
        .unwrap();

        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].relative_path, "readable.docx");
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].relative_path, "nested/victim.docx");
    }
}
