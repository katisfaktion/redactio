use crate::error::AppError;
use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

/// Include both roots of every OTHER saved pair plus app config in other_roots.
/// Call again before each operation; unavailable roots must fail closed.
pub fn validate_roots(
    source: &Path,
    target: &Path,
    other_roots: &[PathBuf],
) -> Result<(PathBuf, PathBuf), AppError> {
    let source = canonical_directory(source)?;
    let target = canonical_directory(target)?;
    check_all_roots(&source, &target, other_roots)?;
    // Both roots must support private metadata; probes never overwrite existing data.
    for root in [&source, &target] {
        fs::read_dir(root)?;
        let probe = tempfile::Builder::new()
            .prefix(".redactio-check-")
            .tempfile_in(root)?;
        probe.close()?;
    }
    Ok((source, target))
}

/// `confirmed` comes from an explicit native host action, never an implicit mkdir.
/// Only creates the final component; does not adopt or remove an existing object.
pub fn create_target(
    source: &Path,
    target: &Path,
    other_roots: &[PathBuf],
    confirmed: bool,
) -> Result<(PathBuf, PathBuf), AppError> {
    if !confirmed {
        return Err(AppError::new("confirmation_required"));
    }
    check_absolute(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| AppError::new("invalid_path"))?;
    let parent = canonical_directory(parent)?;
    let name = target
        .file_name()
        .ok_or_else(|| AppError::new("invalid_path"))?;
    check_name(name)?;
    let target = parent.join(name);
    let source = canonical_directory(source)?;
    check_all_roots(&source, &target, other_roots)?;
    let checked = super::storage::ValidatedWrite::new(&parent, &target)?;
    checked.create_directory()?;
    // A failed final validation leaves the created empty directory for the user.
    validate_roots(&source, &target, other_roots)
}

/// This validates location/type, not pair ownership. The domain caller must check
/// the mapping/output owner before granting a writable root. Use ValidatedWrite
/// to retain identity across validation and replacement; a PathBuf cannot do so.
pub fn validate_write(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    check_absolute(root)?;
    check_absolute(path)?;
    reject_links(root)?;
    let root = canonical_directory(root)?;
    let parent = path.parent().ok_or_else(|| AppError::new("invalid_path"))?;
    reject_links(parent)?;
    let parent = canonical_directory(parent)?;
    if !contains(&root, &parent) {
        return Err(AppError::new("outside_root"));
    }
    let name = path
        .file_name()
        .ok_or_else(|| AppError::new("invalid_path"))?;
    check_name(name)?;
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if is_link(&metadata) || !metadata.is_file() {
                return Err(AppError::new("unsafe_object"));
            }
            super::storage::file_snapshot(&path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

pub(crate) fn check_absolute(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(AppError::new("invalid_path"));
    }
    for component in path.components() {
        if let Component::Normal(name) = component {
            check_name(name)?;
        }
        #[cfg(windows)]
        if let Component::Prefix(prefix) = component {
            use std::path::Prefix;
            if matches!(
                prefix.kind(),
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(..)
            ) {
                return Err(AppError::new("unsupported_path"));
            }
        }
    }
    Ok(())
}

fn check_name(name: &OsStr) -> Result<(), AppError> {
    let text = name.to_str().ok_or_else(|| AppError::new("invalid_path"))?;
    if text.is_empty() || text.contains('\0') {
        return Err(AppError::new("invalid_path"));
    }
    #[cfg(windows)]
    {
        if text.contains([':', '*', '?', '"', '<', '>', '|'])
            || text.ends_with(['.', ' '])
            || text.chars().any(|c| c < ' ')
        {
            return Err(AppError::new("invalid_path"));
        }
        let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        }) {
            return Err(AppError::new("invalid_path"));
        }
    }
    Ok(())
}

pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, AppError> {
    check_absolute(path)?;
    let canonical = fs::canonicalize(path)?;
    if !fs::metadata(&canonical)?.is_dir() {
        return Err(AppError::new("not_directory"));
    }
    #[cfg(windows)]
    {
        super::storage::windows::canonical_directory(&canonical)
    }
    #[cfg(not(windows))]
    {
        Ok(canonical)
    }
}

pub(crate) fn reject_links(path: &Path) -> Result<(), AppError> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if is_link(&metadata) || !metadata.is_dir() {
            return Err(AppError::new("unsafe_object"));
        }
    }
    Ok(())
}

pub(crate) fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn check_all_roots(source: &Path, target: &Path, other_roots: &[PathBuf]) -> Result<(), AppError> {
    let mut roots = vec![source.to_path_buf(), target.to_path_buf()];
    for other in other_roots {
        roots.push(canonical_directory(other)?);
    }
    // ponytail: O(n²) for a few saved folders; use a path index if thousands matter.
    for (index, root) in roots.iter().enumerate() {
        for other in &roots[index + 1..] {
            ensure_separate(root, other)?;
        }
    }
    Ok(())
}

fn ensure_separate(a: &Path, b: &Path) -> Result<(), AppError> {
    if contains(a, b) || contains(b, a) {
        Err(AppError::new("folder_overlap"))
    } else {
        Ok(())
    }
}

fn contains(root: &Path, path: &Path) -> bool {
    let mut parts = path.components();
    root.components().all(|component| {
        parts
            .next()
            .is_some_and(|other| equal(component.as_os_str(), other.as_os_str()))
    })
}

fn equal(a: &OsStr, b: &OsStr) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
        let a: Vec<_> = a.encode_wide().collect();
        let b: Vec<_> = b.encode_wide().collect();
        // Windows ordinal case comparison, not Unicode lowercase/string prefixes.
        unsafe {
            CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1)
                == CSTR_EQUAL
        }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
