use crate::{domain::paths, error::AppError};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

pub struct ResourcePaths {
    pub sidecar_executable: PathBuf,
    pub sidecar_args: Vec<OsString>,
    pub model_root: PathBuf,
}

/// Release builds always use resources beside this executable.
pub fn resolve() -> Result<ResourcePaths, AppError> {
    #[cfg(debug_assertions)]
    {
        let executable = std::env::var_os("REDACTIO_SIDECAR_EXECUTABLE");
        let model_root = std::env::var_os("REDACTIO_MODEL_DIR");
        let arguments = std::env::var("REDACTIO_SIDECAR_ARGS_JSON");
        if executable.is_some() || model_root.is_some() || arguments.is_ok() {
            let executable = PathBuf::from(executable.ok_or_else(setup_error)?);
            let model_root = PathBuf::from(model_root.ok_or_else(setup_error)?);
            if !executable.is_absolute()
                || !executable.is_file()
                || !model_root.is_absolute()
                || !model_root.is_dir()
            {
                return Err(setup_error());
            }
            let arguments: Vec<String> = match arguments {
                Ok(value) => serde_json::from_str(&value).map_err(|_| setup_error())?,
                Err(std::env::VarError::NotPresent) => Vec::new(),
                Err(_) => return Err(setup_error()),
            };
            return Ok(ResourcePaths {
                sidecar_executable: executable,
                sidecar_args: arguments.into_iter().map(OsString::from).collect(),
                model_root,
            });
        }
    }
    resolve_packaged(&std::env::current_exe().map_err(|_| setup_error())?)
}

pub fn resolve_packaged(executable: &Path) -> Result<ResourcePaths, AppError> {
    resource(executable.to_path_buf(), false)?;
    let root = executable.parent().ok_or_else(setup_error)?;
    let sidecar = resource(root.join("sidecar"), true)?;
    resource(sidecar.join("_internal"), true)?;
    let model_root = resource(root.join("models"), true)?;
    for required in [
        "manifest.json",
        "de_core_news_lg/config.cfg",
        "de_core_news_lg/meta.json",
    ] {
        resource(model_root.join(required), false)?;
    }
    Ok(ResourcePaths {
        sidecar_executable: resource(
            sidecar.join(if cfg!(windows) {
                "redactio-sidecar.exe"
            } else {
                "redactio-sidecar"
            }),
            false,
        )?,
        sidecar_args: Vec::new(),
        model_root,
    })
}

pub fn webview_directory(executable: &Path) -> Result<PathBuf, AppError> {
    resource(executable.to_path_buf(), false)?;
    let runtime = resource(
        executable
            .parent()
            .ok_or_else(setup_error)?
            .join("webview2"),
        true,
    )?;
    resource(runtime.join("msedgewebview2.exe"), false)?;
    Ok(runtime)
}

fn resource(path: PathBuf, directory: bool) -> Result<PathBuf, AppError> {
    paths::check_absolute(&path).map_err(|_| setup_error())?;
    paths::reject_links(path.parent().ok_or_else(setup_error)?).map_err(|_| setup_error())?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| setup_error())?;
    if paths::is_link(&metadata)
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(setup_error());
    }
    if directory {
        for entry in walkdir::WalkDir::new(&path).follow_links(false) {
            let metadata = entry
                .map_err(|_| setup_error())?
                .metadata()
                .map_err(|_| setup_error())?;
            if paths::is_link(&metadata) || !(metadata.is_dir() || metadata.is_file()) {
                return Err(setup_error());
            }
        }
    }
    Ok(path)
}

fn setup_error() -> AppError {
    AppError::new("setup_incomplete")
}
