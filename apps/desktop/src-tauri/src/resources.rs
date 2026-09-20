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
        "biomedbert-de/config.json",
        "biomedbert-de/redactio-model.json",
        "biomedbert-de/model.safetensors",
        "biomedbert-de/tokenizer.json",
        "biomedbert-de/tokenizer_config.json",
        "biomedbert-de/special_tokens_map.json",
        "biomedbert-de/vocab.txt",
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

/// Metadata-only local availability. This never imports or downloads a model.
pub fn list_models(root: &Path) -> Result<Vec<crate::protocol::ModelInfo>, AppError> {
    const BIOMEDBERT: &str = "OpenMed-PII-German-BiomedBERT-Large-340M-v1";
    const BIOMEDBERT_VERSION: &str = "ce797d58600cc20bba9a2500dafc0b7f5c3270c1";

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Manifest {
        models: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        name: String,
        version: String,
        path: PathBuf,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ModelMetadata {
        name: String,
        version: String,
        repository: String,
    }
    #[derive(serde::Deserialize)]
    struct BertConfig {
        architectures: Vec<String>,
        model_type: String,
        max_position_embeddings: u64,
        id2label: serde_json::Value,
    }
    let invalid = || AppError::new("invalid_model_manifest");
    let manifest = resource(root.join("manifest.json"), false).map_err(|_| invalid())?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(manifest).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    let mut names = std::collections::HashSet::new();
    let mut biomedbert = None;
    for entry in manifest.models {
        if entry.name.is_empty()
            || entry.version.is_empty()
            || !names.insert(entry.name.clone())
            || entry.path.as_os_str().is_empty()
            || entry
                .path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(invalid());
        }
        if entry.name == BIOMEDBERT {
            biomedbert = Some(entry);
        }
    }
    let Some(entry) = biomedbert else {
        return Ok(Vec::new());
    };
    let model = root.join(&entry.path);
    let labels = (|| {
        if entry.version != BIOMEDBERT_VERSION || model != root.join("biomedbert-de") {
            return Err(invalid());
        }
        for required in [
            "model.safetensors",
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
            "vocab.txt",
        ] {
            resource(model.join(required), false)?;
        }
        let config = resource(model.join("config.json"), false)?;
        let config: BertConfig =
            serde_json::from_slice(&fs::read(config)?).map_err(|_| invalid())?;
        let metadata = resource(model.join("redactio-model.json"), false)?;
        let metadata: ModelMetadata =
            serde_json::from_slice(&fs::read(metadata)?).map_err(|_| invalid())?;
        if metadata.name != BIOMEDBERT
            || metadata.version != BIOMEDBERT_VERSION
            || metadata.repository != format!("OpenMed/{BIOMEDBERT}")
            || config.architectures != ["BertForTokenClassification"]
            || config.model_type != "bert"
            || config.max_position_embeddings != 512
        {
            return Err(invalid());
        }
        model_entity_types(config.id2label).ok_or_else(invalid)
    })();
    let (compatible, entity_types) = match labels {
        Ok(entity_types) => (true, entity_types),
        Err(_) => (false, Vec::new()),
    };
    Ok(vec![crate::protocol::ModelInfo {
        name: entry.name,
        version: entry.version,
        compatible,
        entity_types,
    }])
}

fn model_entity_types(
    id2label: serde_json::Value,
) -> Option<Vec<crate::domain::settings::EntityType>> {
    let labels = id2label.as_object()?;
    if labels.is_empty() || labels.keys().any(|key| key.parse::<u64>().is_err()) {
        return None;
    }
    let mut entity_types = labels
        .values()
        .map(serde_json::Value::as_str)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .filter_map(|label| match label {
            "O" => None,
            label => Some(
                label
                    .strip_prefix("B-")
                    .or_else(|| label.strip_prefix("I-"))
                    .unwrap_or(label),
            ),
        })
        .map(|label| crate::domain::settings::EntityType::parse(label.into()).ok())
        .collect::<Option<Vec<_>>>()?;
    entity_types.sort_unstable();
    entity_types.dedup();
    (!entity_types.is_empty()).then_some(entity_types)
}
