use std::{ffi::OsString, path::PathBuf};

pub struct ResourcePaths {
    pub sidecar_executable: PathBuf,
    pub sidecar_args: Vec<OsString>,
    pub model_root: PathBuf,
}

pub fn resolve() -> ResourcePaths {
    if let (Some(executable), Some(model_root)) = (
        std::env::var_os("REDACTIO_SIDECAR_EXECUTABLE"),
        std::env::var_os("REDACTIO_MODEL_DIR"),
    ) {
        let args = std::env::var("REDACTIO_SIDECAR_ARGS_JSON")
            .ok()
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .unwrap_or_default()
            .into_iter()
            .map(OsString::from)
            .collect();
        return ResourcePaths {
            sidecar_executable: executable.into(),
            sidecar_args: args,
            model_root: model_root.into(),
        };
    }

    let package_root = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_default();
    ResourcePaths {
        sidecar_executable: package_root.join("sidecar").join(if cfg!(windows) {
            "redactio-sidecar.exe"
        } else {
            "redactio-sidecar"
        }),
        sidecar_args: Vec::new(),
        model_root: package_root.join("models"),
    }
}
