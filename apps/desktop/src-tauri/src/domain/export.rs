use super::paths::{canonical_directory, ensure_separate, reject_links};
use crate::error::AppError;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Supply both roots of every saved pair; unavailable roots fail closed.
pub fn validate_export_destination(
    destination: &Path,
    roots: &[PathBuf],
    config_dir: &Path,
) -> Result<PathBuf, AppError> {
    let canonical = canonical_directory(destination)?;
    reject_links(destination)?;
    for root in roots.iter().map(PathBuf::as_path).chain([config_dir]) {
        ensure_separate(&canonical, &canonical_directory(root)?)?;
    }
    if fs::read_dir(&canonical)?.next().transpose()?.is_some() {
        return Err(AppError::new("export_not_empty"));
    }
    Ok(canonical)
}
