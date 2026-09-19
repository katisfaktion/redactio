use super::paths::{is_link, reject_links, validate_write};
use crate::error::AppError;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FileSnapshot {
    identity: (u64, u64),
    size: u64,
    modified: SystemTime,
}

/// Host-only, single-use write authority. The caller must first verify pair/root
/// separation and record ownership; filesystem validation cannot infer ownership.
/// Retains original object/directory identities until the replacement is complete.
#[derive(Debug)]
pub struct ValidatedWrite {
    path: PathBuf,
    directories: Vec<(PathBuf, (u64, u64))>,
    destination: Option<FileSnapshot>,
}

impl ValidatedWrite {
    pub fn new(root: &Path, path: &Path) -> Result<Self, AppError> {
        let path = validate_write(root, path)?;
        let directories = path
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?
            .ancestors()
            .map(|path| Ok((path.to_path_buf(), identity(path)?.0)))
            .collect::<Result<Vec<_>, AppError>>()?;
        let destination = optional_snapshot(&path)?;
        Ok(Self {
            path,
            directories,
            destination,
        })
    }

    pub fn validate(&self) -> Result<(), AppError> {
        for (path, expected) in &self.directories {
            let metadata = fs::symlink_metadata(path).map_err(|_| AppError::new("path_changed"))?;
            if !metadata.is_dir()
                || is_link(&metadata)
                || identity(path).map_err(|_| AppError::new("path_changed"))?.0 != *expected
            {
                return Err(AppError::new("path_changed"));
            }
        }
        if optional_snapshot(&self.path).map_err(|_| AppError::new("path_changed"))?
            != self.destination
        {
            return Err(AppError::new("path_changed"));
        }
        Ok(())
    }

    pub fn write_atomic(self, bytes: &[u8]) -> Result<(), AppError> {
        self.validate()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?;
        // Windows handles deny delete-sharing, pinning every ancestor while IO runs.
        let directories = self
            .directories
            .iter()
            .map(|(path, _)| open_directory(path))
            .collect::<Result<Vec<_>, _>>()?;
        self.validate()?;
        let directory = &directories[0];
        let io_parent = anchored_parent(parent, directory);
        let destination = io_parent.join(
            self.path
                .file_name()
                .ok_or_else(|| AppError::new("invalid_path"))?,
        );
        let mut temporary = tempfile::Builder::new()
            .prefix(".redactio-write-")
            .tempfile_in(&io_parent)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        let expected_temp = snapshot_file(temporary.as_file())?;
        self.validate()?;
        if file_snapshot(temporary.path())? != expected_temp {
            return Err(AppError::new("path_changed"));
        }
        // Close the file before ReplaceFileW, which opens the replacement exclusively.
        let temporary = temporary.into_temp_path();
        replace(&temporary, &destination, self.destination.as_ref())?;
        // TempPath removes only this operation's random temporary path, if still present.
        #[cfg(unix)]
        directory
            .sync_all()
            .map_err(|_| AppError::new("storage_durability_uncertain"))?;
        Ok(())
    }

    pub(crate) fn create_directory(self) -> Result<(), AppError> {
        if self.destination.is_some() {
            return Err(AppError::new("path_exists"));
        }
        self.validate()?;
        let directories = self
            .directories
            .iter()
            .map(|(path, _)| open_directory(path))
            .collect::<Result<Vec<_>, _>>()?;
        self.validate()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?;
        let path = anchored_parent(parent, &directories[0]).join(
            self.path
                .file_name()
                .ok_or_else(|| AppError::new("invalid_path"))?,
        );
        fs::create_dir(&path)?;
        reject_links(&self.path)?;
        Ok(())
    }

    fn open_read(self) -> Result<File, AppError> {
        self.validate()?;
        let directories = self
            .directories
            .iter()
            .map(|(path, _)| open_directory(path))
            .collect::<Result<Vec<_>, _>>()?;
        self.validate()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?;
        let path = anchored_parent(parent, &directories[0]).join(
            self.path
                .file_name()
                .ok_or_else(|| AppError::new("invalid_path"))?,
        );
        let file = open_regular_file(&path)?;
        if Some(snapshot_file(&file)?) != self.destination {
            return Err(AppError::new("path_changed"));
        }
        Ok(file)
    }
}

pub(crate) fn open_validated_read(root: &Path, path: &Path) -> Result<File, AppError> {
    ValidatedWrite::new(root, path)?.open_read()
}

/// Convenience for an already authorized host path. Takes a fresh identity
/// snapshot; use ValidatedWrite::new(root, path) when authorization happened earlier.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| AppError::new("invalid_path"))?;
    ValidatedWrite::new(parent, path)?.write_atomic(bytes)
}

/// Fresh-start recovery only: retain the entire private metadata directory without
/// replacing another backup. Source documents and the old target are never moved.
pub(crate) fn retain_private_metadata(source: &Path, backup: &Path) -> Result<(), AppError> {
    reject_links(source)?;
    reject_links(backup)?;
    if backup.parent() != Some(source) {
        return Err(AppError::new("outside_root"));
    }
    let from = source.join("_redactio");
    let to = backup.join("_redactio");
    let destination = ValidatedWrite::new(source, &to)?;
    reject_links(&from)?;
    let expected = identity(&from)?.0;
    let ancestors = source
        .ancestors()
        .map(open_directory)
        .collect::<Result<Vec<_>, _>>()?;
    let source_handle = &ancestors[0];
    let backup_handle = open_directory(backup)?;
    destination.validate()?;
    let from = anchored_parent(source, source_handle).join("_redactio");
    let to = anchored_parent(backup, &backup_handle).join("_redactio");
    if identity(&from)?.0 != expected {
        return Err(AppError::new("path_changed"));
    }
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from =
            CString::new(from.as_os_str().as_bytes()).map_err(|_| AppError::new("invalid_path"))?;
        let to =
            CString::new(to.as_os_str().as_bytes()).map_err(|_| AppError::new("invalid_path"))?;
        if unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        source_handle
            .sync_all()
            .map_err(|_| AppError::new("storage_durability_uncertain"))?;
        backup_handle
            .sync_all()
            .map_err(|_| AppError::new("storage_durability_uncertain"))?;
    }
    #[cfg(windows)]
    windows::move_no_replace(&from, &to)?;
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        return Err(AppError::new("unsupported_platform"));
    }
    Ok(())
}

fn optional_snapshot(path: &Path) -> Result<Option<FileSnapshot>, AppError> {
    match fs::symlink_metadata(path) {
        Ok(_) => file_snapshot(path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn file_snapshot(path: &Path) -> Result<FileSnapshot, AppError> {
    snapshot_file(&open_regular_file(path)?)
}

fn open_regular_file(path: &Path) -> Result<File, AppError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    snapshot_file(&file)?;
    Ok(file)
}

fn snapshot_file(file: &File) -> Result<FileSnapshot, AppError> {
    let metadata = file.metadata()?;
    if is_link(&metadata) || !metadata.is_file() {
        return Err(AppError::new("unsafe_object"));
    }
    #[cfg(unix)]
    let (identity, links) = {
        use std::os::unix::fs::MetadataExt;
        ((metadata.dev(), metadata.ino()), metadata.nlink())
    };
    #[cfg(windows)]
    let (identity, links) = windows::file_identity(file)?;
    if links != 1 {
        return Err(AppError::new("unsafe_object"));
    }
    Ok(FileSnapshot {
        identity,
        size: metadata.len(),
        modified: metadata.modified()?,
    })
}

fn identity(path: &Path) -> Result<((u64, u64), u64), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path)?;
        Ok(((metadata.dev(), metadata.ino()), metadata.nlink()))
    }
    #[cfg(windows)]
    {
        windows::identity(path)
    }
}

fn open_directory(path: &Path) -> Result<File, AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        Ok(fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?)
    }
    #[cfg(windows)]
    {
        windows::open_directory(path)
    }
}

fn anchored_parent(parent: &Path, directory: &File) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // Linux development: directory-relative IO also survives a concurrent rename.
        let _ = parent;
        PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = directory;
        parent.to_path_buf()
    }
}

fn replace(
    temporary: &Path,
    destination: &Path,
    expected: Option<&FileSnapshot>,
) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        if optional_snapshot(destination)?.as_ref() != expected {
            return Err(AppError::new("path_changed"));
        }
        if expected.is_some() {
            fs::rename(temporary, destination)?;
        } else {
            // link is an atomic no-replace publish; never overwrite a later object.
            fs::hard_link(temporary, destination)?;
            fs::remove_file(temporary)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows::replace(temporary, destination, expected)
    }
}

#[cfg(windows)]
pub(crate) mod windows {
    use super::{optional_snapshot, FileSnapshot};
    use crate::error::AppError;
    use std::{
        ffi::OsString,
        fs::{self, File, OpenOptions},
        io,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            fs::OpenOptionsExt,
            io::AsRawHandle,
        },
        path::{Path, PathBuf},
    };
    use windows_sys::Win32::Storage::FileSystem::*;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn open_directory(path: &Path) -> Result<File, AppError> {
        Ok(OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?)
    }

    pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, AppError> {
        let file = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let mut buffer = vec![0u16; 32768];
        // Resolve drive-letter/SUBST/8.3 aliases to a normalized volume-GUID path.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_GUID,
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if length as usize >= buffer.len() {
            return Err(AppError::new("invalid_path"));
        }
        Ok(PathBuf::from(OsString::from_wide(
            &buffer[..length as usize],
        )))
    }

    pub(super) fn identity(path: &Path) -> Result<((u64, u64), u64), AppError> {
        let file = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        file_identity(&file)
    }

    pub(super) fn file_identity(file: &File) -> Result<((u64, u64), u64), AppError> {
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let info = unsafe { info.assume_init() };
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(AppError::new("unsafe_object"));
        }
        Ok((
            (
                info.dwVolumeSerialNumber as u64,
                (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64,
            ),
            info.nNumberOfLinks as u64,
        ))
    }

    pub(super) fn move_no_replace(from: &Path, to: &Path) -> Result<(), AppError> {
        if unsafe {
            MoveFileExW(
                wide(from).as_ptr(),
                wide(to).as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }

    pub(super) fn replace(
        temporary: &Path,
        destination: &Path,
        expected: Option<&FileSnapshot>,
    ) -> Result<(), AppError> {
        let temporary_wide = wide(temporary);
        let destination_wide = wide(destination);
        if expected.is_none() {
            // Without REPLACE_EXISTING this cannot overwrite a concurrent creation.
            if unsafe {
                MoveFileExW(
                    temporary_wide.as_ptr(),
                    destination_wide.as_ptr(),
                    MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(io::Error::last_os_error().into());
            }
            return Ok(());
        }
        let parent = destination
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?;
        // ReplaceFileW can lose the original on error 1176 without a backup.
        // Own a private, random recovery directory; never share a fixed backup name.
        let recovery = tempfile::Builder::new()
            .prefix(".redactio-recovery-")
            .tempdir_in(parent)?;
        let backup = recovery.path().join("original");
        let backup_wide = wide(&backup);
        if optional_snapshot(destination)?.as_ref() != expected {
            return Err(AppError::new("path_changed"));
        }
        let replaced = unsafe {
            ReplaceFileW(
                destination_wide.as_ptr(),
                temporary_wide.as_ptr(),
                backup_wide.as_ptr(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if replaced != 0 {
            recovery
                .close()
                .map_err(|_| AppError::new("storage_cleanup_required"))?;
            return Ok(());
        }
        let error = io::Error::last_os_error();
        restore_backup(recovery, destination)?;
        Err(error.into())
    }

    fn restore_backup(recovery: tempfile::TempDir, destination: &Path) -> Result<(), AppError> {
        let backup = recovery.path().join("original");
        match fs::symlink_metadata(&backup) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                let _retained = recovery.keep();
                return Err(AppError::new("storage_recovery_required"));
            }
            Ok(_) => (),
        }
        // A partial ReplaceFileW failure may have moved the original here. Never
        // overwrite a later destination or let TempDir discard recoverable bytes.
        let restored = unsafe {
            MoveFileExW(
                wide(&backup).as_ptr(),
                wide(destination).as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if restored == 0 {
            let _retained = recovery.keep();
            return Err(AppError::new("storage_recovery_required"));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn partial_replacement_restores_original_without_overwriting_later_data() {
            for occupied in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let recovery = tempfile::tempdir_in(root.path()).unwrap();
                let backup = recovery.path().join("original");
                fs::write(&backup, b"original").unwrap();
                let destination = root.path().join("state.json");
                if occupied {
                    fs::write(&destination, b"later").unwrap();
                }
                let result = restore_backup(recovery, &destination);
                if occupied {
                    assert_eq!(result.unwrap_err().code, "storage_recovery_required");
                    assert_eq!(fs::read(&backup).unwrap(), b"original");
                    assert_eq!(fs::read(&destination).unwrap(), b"later");
                } else {
                    result.unwrap();
                    assert_eq!(fs::read(&destination).unwrap(), b"original");
                    assert!(!backup.exists());
                }
            }
        }
    }
}
