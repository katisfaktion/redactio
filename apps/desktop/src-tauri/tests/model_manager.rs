mod common;
use redactio_lib::{
    domain::sync::RunController, model_manager::ModelManager, model_store::ModelUseGuard,
    resources::ResourcePaths,
};
use std::{path::Path, time::Duration};

#[test]
fn model_use_lease_blocks_removal_until_released() {
    let root = tempfile::tempdir().unwrap();
    let name = common::fixture_model_store(root.path());
    let receipt =
        std::fs::File::open(root.path().join("fixture-model/redactio-model.json")).unwrap();
    let lease = ModelUseGuard::acquire(root.path(), &name).unwrap();
    assert!(receipt.try_lock().is_err());
    drop(lease);
    receipt.try_lock().unwrap();
    assert!(ModelUseGuard::acquire(root.path(), &name).is_err());
    receipt.unlock().unwrap();
}

fn manager(root: &Path, mode: &str) -> ModelManager {
    ModelManager::new(ResourcePaths {
        sidecar_executable: std::env::var_os("REDACTIO_TEST_PYTHON").unwrap().into(),
        sidecar_args: vec![
            common::manifest_dir()
                .join("tests/fake_model_worker.py")
                .into_os_string(),
            mode.into(),
        ],
        model_root: root.into(),
    })
}
fn fixture(root: &Path) -> String {
    let name = common::fixture_model_store(root);
    let registry = std::fs::read(root.join("manifest.json")).unwrap();
    std::fs::write(root.join("fixture-registry.json"), &registry).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&registry).unwrap();
    value["models"][0]["state"] = "available".into();
    value["models"][0]["path"] = serde_json::Value::Null;
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    name
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
async fn terminal(manager: &ModelManager, id: uuid::Uuid) -> redactio_lib::model_store::ModelJob {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let job = manager.get_job(id).unwrap().unwrap();
            if matches!(
                job.stage,
                redactio_lib::model_store::ModelJobStage::Ready
                    | redactio_lib::model_store::ModelJobStage::Failed
                    | redactio_lib::model_store::ModelJobStage::Cancelled
                    | redactio_lib::model_store::ModelJobStage::Removed
            ) {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[test]
fn install_requires_matching_plan_verified_result_and_registry() {
    runtime().block_on(async {
        for mode in [
            "ok",
            "no-publish",
            "malformed",
            "oversized",
            "wrong-id",
            "alternate-id",
            "wrong-model",
            "bad-total",
            "early-exit",
            "wrong-result",
        ] {
            let root = tempfile::tempdir().unwrap();
            let config = tempfile::tempdir().unwrap();
            let name = fixture(root.path());
            let manager = manager(root.path(), mode);
            let runs = RunController::new(config.path().join("settings.json"));
            let checked = manager
                .check(
                    redactio_lib::model_store::ModelSource::Receipt { name },
                    &runs,
                )
                .await
                .unwrap();
            assert!(manager
                .start_install(uuid::Uuid::new_v4(), &runs, config.path(), None, |_| {})
                .await
                .is_err());
            let id = manager
                .start_install(
                    checked.plan_id.parse().unwrap(),
                    &runs,
                    config.path(),
                    None,
                    |_| {},
                )
                .await
                .unwrap();
            let job = terminal(&manager, id).await;
            assert_eq!(
                job.stage,
                if mode == "ok" {
                    redactio_lib::model_store::ModelJobStage::Ready
                } else {
                    redactio_lib::model_store::ModelJobStage::Failed
                },
                "{mode}"
            );
            assert!(runs.try_operation().is_ok(), "{mode}");
            assert_eq!(manager.get_job(id).unwrap(), Some(job));
        }
    });
}
#[test]
fn cancellation_reaps_worker_and_reconciles_publication() {
    runtime().block_on(async {
        for mode in ["block", "publish-block"] {
            let root = tempfile::tempdir().unwrap();
            let config = tempfile::tempdir().unwrap();
            let name = fixture(root.path());
            let manager = manager(root.path(), mode);
            let runs = RunController::new(config.path().join("settings.json"));
            let checked = manager
                .check(
                    redactio_lib::model_store::ModelSource::Receipt { name },
                    &runs,
                )
                .await
                .unwrap();
            let id = manager
                .start_install(
                    checked.plan_id.parse().unwrap(),
                    &runs,
                    config.path(),
                    None,
                    |_| {},
                )
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !root.path().join("started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(runs.try_operation().is_err());
            assert!(
                redactio_lib::domain::mapping::CollectionGuard::acquire(config.path()).is_err()
            );
            assert!(manager
                .start_install(
                    checked.plan_id.parse().unwrap(),
                    &runs,
                    config.path(),
                    None,
                    |_| {}
                )
                .await
                .is_err());
            manager.cancel(id).await.unwrap();
            assert_eq!(probe(root.path(), "process-running"), "exited");
            assert_eq!(
                terminal(&manager, id).await.stage,
                if mode == "block" {
                    redactio_lib::model_store::ModelJobStage::Cancelled
                } else {
                    redactio_lib::model_store::ModelJobStage::Ready
                }
            );
            assert!(runs.try_operation().is_ok());
        }
    });
}

fn probe(root: &Path, mode: &str) -> String {
    let output = std::process::Command::new(std::env::var_os("REDACTIO_TEST_PYTHON").unwrap())
        .arg(common::manifest_dir().join("tests/fake_model_worker.py"))
        .arg(mode)
        .arg("--model-dir")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
#[test]
fn rust_writer_lock_blocks_python_worker_and_standalone_preparation() {
    let root = tempfile::tempdir().unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.path().join(".redactio-models-lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(probe(root.path(), "writer-probe"), "model_store_busy");
    assert_eq!(probe(root.path(), "prepare-probe"), "model_store_busy");
    lock.unlock().unwrap();
    assert_eq!(probe(root.path(), "writer-probe"), "acquired");
}
#[test]
fn rust_lease_blocks_actual_python_removal_then_releases() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let name = common::fixture_model_store(root.path());
        let lease = ModelUseGuard::acquire(root.path(), &name).unwrap();
        let manager = manager(root.path(), "real");
        let runs = RunController::new(config.path().join("settings.json"));
        let id = manager
            .remove(&name, &runs, config.path(), None, |_| {})
            .await
            .unwrap();
        let result = terminal(&manager, id).await;
        assert_eq!(
            result.stage,
            redactio_lib::model_store::ModelJobStage::Failed
        );
        assert_eq!(result.error.unwrap().code, "model_in_use");
        assert!(root
            .path()
            .join("fixture-model/redactio-model.json")
            .is_file());
        drop(lease);
        let id = manager
            .remove(&name, &runs, config.path(), None, |_| {})
            .await
            .unwrap();
        assert_eq!(
            terminal(&manager, id).await.stage,
            redactio_lib::model_store::ModelJobStage::Removed
        );
        assert!(!root.path().join("fixture-model").exists());
    });
}
#[test]
fn removal_retries_persisted_removing_and_guards_all_saved_pairs() {
    runtime().block_on(async {
        use redactio_lib::domain::settings::{save_settings, Settings};
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let name = common::fixture_model_store(root.path());
        let manager = manager(root.path(), "real");
        let runs = RunController::new(config.path().join("settings.json"));
        let mut settings = Settings::default();
        for label in ["first", "unselected"] {
            let source = root.path().join(format!("{label}-source"));
            let target = root.path().join(format!("{label}-target"));
            std::fs::create_dir(&source).unwrap();
            std::fs::create_dir(&target).unwrap();
            settings.add(label, &source, &target).unwrap();
        }
        settings.sync_pairs[1].config.model = name.clone();
        settings.selected_sync_pair_id = Some(settings.sync_pairs[0].id);
        save_settings(&config.path().join("settings.json"), &settings).unwrap();
        let models = manager.list(&config.path().join("settings.json")).unwrap();
        assert_eq!(
            models
                .iter()
                .find(|model| model.name == name)
                .unwrap()
                .used_by_pairs[0]
                .name,
            "unselected"
        );
        assert_eq!(
            manager
                .remove(&name, &runs, config.path(), None, |_| {})
                .await
                .unwrap_err()
                .code,
            "model_in_use"
        );
        settings.sync_pairs[1].config.model = "other".into();
        save_settings(&config.path().join("settings.json"), &settings).unwrap();
        let mut registry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.path().join("manifest.json")).unwrap())
                .unwrap();
        registry["models"][0]["state"] = "removing".into();
        std::fs::write(
            root.path().join("manifest.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let id = manager
            .remove(&name, &runs, config.path(), None, |_| {})
            .await
            .unwrap();
        assert_eq!(
            terminal(&manager, id).await.stage,
            redactio_lib::model_store::ModelJobStage::Removed
        );
    });
}
#[test]
fn listing_empty_store_never_starts_worker_and_malformed_store_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("models");
    let manager = ModelManager::new(ResourcePaths {
        sidecar_executable: root.path().join("missing-worker"),
        sidecar_args: vec![],
        model_root: missing.clone(),
    });
    assert_eq!(
        manager
            .list(&root.path().join("settings.json"))
            .unwrap()
            .len(),
        2
    );
    assert!(!missing.exists());
    std::fs::create_dir(&missing).unwrap();
    std::fs::write(missing.join("manifest.json"), b"broken").unwrap();
    assert_eq!(
        manager
            .list(&root.path().join("settings.json"))
            .unwrap_err()
            .code,
        "invalid_model_manifest"
    );
}

#[test]
fn legacy_lease_uses_existing_read_only_receipt_without_creating_files() {
    use redactio_lib::model_store;
    let root = tempfile::tempdir().unwrap();
    let entry = model_store::catalog_models().unwrap().remove(0);
    let model = root.path().join(&entry.directory);
    std::fs::create_dir(&model).unwrap();
    for file in &entry.descriptor.files {
        std::fs::write(model.join(&file.filename), b"fixture").unwrap();
    }
    std::fs::write(model.join("config.json"), br#"{"architectures":["BertForTokenClassification"],"model_type":"bert","max_position_embeddings":512,"id2label":{"0":"O","1":"B-DATE"}}"#).unwrap();
    std::fs::write(
        model.join("tokenizer_config.json"),
        br#"{"model_max_length":512}"#,
    )
    .unwrap();
    let receipt = model.join("redactio-model.json");
    std::fs::write(&receipt, serde_json::to_vec(&serde_json::json!({"name":entry.descriptor.name,"version":entry.descriptor.version,"repository":entry.descriptor.repository})).unwrap()).unwrap();
    std::fs::write(root.path().join("manifest.json"), serde_json::to_vec(&serde_json::json!({"models":[{"name":entry.descriptor.name,"version":entry.descriptor.version,"path":entry.directory}]})).unwrap()).unwrap();
    let before = std::fs::read(root.path().join("manifest.json")).unwrap();
    let mut permissions = std::fs::metadata(&receipt).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&receipt, permissions).unwrap();
    let lease = ModelUseGuard::acquire(root.path(), &entry.descriptor.name).unwrap();
    assert_eq!(
        std::fs::read(root.path().join("manifest.json")).unwrap(),
        before
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    drop(lease);
    #[cfg(windows)]
    {
        let mut permissions = std::fs::metadata(&receipt).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&receipt, permissions).unwrap();
    }
}

#[test]
fn newer_check_invalidates_old_plan_and_ready_state_precedes_event() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let name = fixture(root.path());
        let manager = manager(root.path(), "ok");
        let runs = RunController::new(config.path().join("settings.json"));
        let source = redactio_lib::model_store::ModelSource::Receipt { name };
        let first = manager.check(source.clone(), &runs).await.unwrap();
        let second = manager.check(source, &runs).await.unwrap();
        assert_eq!(
            manager
                .start_install(
                    first.plan_id.parse().unwrap(),
                    &runs,
                    config.path(),
                    None,
                    |_| {}
                )
                .await
                .unwrap_err()
                .code,
            "model_plan_expired"
        );
        let observer = manager.clone();
        let (sent, mut events) = tokio::sync::mpsc::unbounded_channel();
        let id = manager
            .start_install(
                second.plan_id.parse().unwrap(),
                &runs,
                config.path(),
                None,
                move |job| {
                    assert_eq!(
                        observer.get_job(job.job_id.parse().unwrap()).unwrap(),
                        Some(job.clone())
                    );
                    sent.send(job).unwrap();
                },
            )
            .await
            .unwrap();
        while let Some(job) = events.recv().await {
            if job.stage == redactio_lib::model_store::ModelJobStage::Ready {
                break;
            }
        }
        assert_eq!(
            terminal(&manager, id).await.stage,
            redactio_lib::model_store::ModelJobStage::Ready
        );
    });
}

#[test]
fn model_lease_rejects_a_hardlinked_identity_receipt() {
    let root = tempfile::tempdir().unwrap();
    let name = common::fixture_model_store(root.path());
    std::fs::hard_link(
        root.path().join("fixture-model/redactio-model.json"),
        root.path().join("receipt-alias"),
    )
    .unwrap();
    assert!(ModelUseGuard::acquire(root.path(), &name).is_err());
}

#[test]
fn partial_removal_and_invalid_installations_report_remaining_disk_bytes() {
    use redactio_lib::model_store::{list_managed, ManagedState};
    for removing in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let name = common::fixture_model_store(root.path());
        let directory = root.path().join("fixture-model");
        if removing {
            std::fs::remove_file(directory.join("model.safetensors")).unwrap();
            let mut registry: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.path().join("manifest.json")).unwrap())
                    .unwrap();
            registry["models"][0]["state"] = "removing".into();
            std::fs::write(
                root.path().join("manifest.json"),
                serde_json::to_vec(&registry).unwrap(),
            )
            .unwrap();
        } else {
            std::fs::write(directory.join("config.json"), b"invalid").unwrap();
        }
        let expected: u64 = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(expected > 0);
        let model = list_managed(root.path())
            .unwrap()
            .into_iter()
            .find(|model| model.name == name)
            .unwrap();
        assert_eq!(
            model.state,
            if removing {
                ManagedState::Removing
            } else {
                ManagedState::Invalid
            }
        );
        assert_eq!(model.installed_bytes, expected);
    }
}
