mod common;

use redactio_lib::{
    domain::{
        detection,
        mapping::CollectionGuard,
        settings::{
            load_settings, save_settings, CustomRule, EntityType, ProcessingConfig, Settings,
        },
        sync::RunController,
    },
    protocol::{PreviewRulesPayload, PreviewRulesResult},
    sidecar::Sidecar,
};
use std::{fs, path::PathBuf, time::Duration};
use uuid::Uuid;

struct Fixture {
    root: tempfile::TempDir,
    path: PathBuf,
    settings: Settings,
    controller: RunController,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        let mut settings = Settings::default();
        for name in ["A", "B"] {
            let source = root.path().join(format!("{name}-in"));
            let target = root.path().join(format!("{name}-out"));
            fs::create_dir(&source).unwrap();
            fs::create_dir(&target).unwrap();
            settings.add(name, &source, &target).unwrap();
        }
        let path = config.join("settings.json");
        save_settings(&path, &settings).unwrap();
        Self {
            controller: RunController::new(path.clone()),
            path,
            settings,
            root,
        }
    }
    fn sidecar(&self) -> Sidecar {
        Sidecar::new(
            std::env::var_os("REDACTIO_TEST_PYTHON").unwrap().into(),
            vec![
                common::manifest_dir()
                    .join("tests/fake_sidecar.py")
                    .into_os_string(),
                "detection".into(),
                self.root.path().join("engine-version").into_os_string(),
            ],
            self.root.path().into(),
        )
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn rules(pattern: &str) -> ProcessingConfig {
    ProcessingConfig {
        enabled_entities: vec![],
        custom_rules: vec![CustomRule::Regex {
            id: Uuid::new_v4(),
            entity_type: EntityType::Custom,
            enabled: true,
            pattern: pattern.into(),
        }],
        ..ProcessingConfig::default()
    }
}

#[test]
fn validated_saves_preserve_other_pairs_and_failed_or_equivalent_save_bytes() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar();
        let a = f.settings.sync_pairs[0].id;
        let updated = detection::save_processing_config(&f.controller, &sidecar, a, rules("Anna"))
            .await
            .unwrap();
        assert_eq!(updated.sync_pairs[1], f.settings.sync_pairs[1]);
        assert_ne!(
            updated.sync_pairs[0].processing_revision,
            f.settings.sync_pairs[0].processing_revision
        );
        let bytes = fs::read(&f.path).unwrap();
        detection::save_processing_config(
            &f.controller,
            &sidecar,
            a,
            updated.sync_pairs[0].config.clone(),
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&f.path).unwrap(), bytes);
        for config in [
            rules("["),
            ProcessingConfig {
                model: "missing".into(),
                ..rules("Anna")
            },
        ] {
            assert!(
                detection::save_processing_config(&f.controller, &sidecar, a, config)
                    .await
                    .is_err()
            );
            assert_eq!(fs::read(&f.path).unwrap(), bytes);
        }
        let restored: PreviewRulesResult = sidecar
            .request(
                "preview_rules",
                &PreviewRulesPayload {
                    sync_pair_id: a,
                    processing_revision: updated.sync_pairs[0].processing_revision,
                    text: "Anna Bea".into(),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(restored.detections.len(), 1);
        assert_eq!(
            (restored.detections[0].start, restored.detections[0].end),
            (0, 4)
        );
        sidecar.shutdown().await;
    });
}

#[test]
fn preview_restores_saved_snapshot_and_never_changes_settings() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar();
        let a = f.settings.sync_pairs[0].id;
        let saved = detection::save_processing_config(&f.controller, &sidecar, a, rules("Anna"))
            .await
            .unwrap();
        let bytes = fs::read(&f.path).unwrap();
        let result =
            detection::preview_rules(&f.controller, &sidecar, a, rules("Bea"), "Anna Bea".into())
                .await
                .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!((result[0].start, result[0].end), (5, 8));
        assert_eq!(fs::read(&f.path).unwrap(), bytes);
        let restored: PreviewRulesResult = sidecar
            .request(
                "preview_rules",
                &PreviewRulesPayload {
                    sync_pair_id: a,
                    processing_revision: saved.sync_pairs[0].processing_revision,
                    text: "Anna Bea".into(),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(
            (restored.detections[0].start, restored.detections[0].end),
            (0, 4)
        );
        assert!(
            detection::preview_rules(&f.controller, &sidecar, a, rules("["), "Anna".into())
                .await
                .is_err()
        );
        assert_eq!(fs::read(&f.path).unwrap(), bytes);
        sidecar.shutdown().await;
    });
}

#[test]
fn activation_refreshes_fingerprint_once_and_keeps_unselected_pair_unchanged() {
    runtime().block_on(async {
        let f = Fixture::new();
        let sidecar = f.sidecar();
        let a = f.settings.sync_pairs[0].id;
        let first = detection::refresh_processing_config(&f.controller, &sidecar, a)
            .await
            .unwrap();
        let revision = first.sync_pairs[0].processing_revision;
        assert_ne!(revision, f.settings.sync_pairs[0].processing_revision);
        assert_eq!(first.sync_pairs[1], f.settings.sync_pairs[1]);
        let repeated = detection::refresh_processing_config(&f.controller, &sidecar, a)
            .await
            .unwrap();
        assert_eq!(repeated, first);
        fs::write(f.root.path().join("engine-version"), "upgrade-2").unwrap();
        let upgraded = detection::refresh_processing_config(&f.controller, &sidecar, a)
            .await
            .unwrap();
        assert_ne!(upgraded.sync_pairs[0].processing_revision, revision);
        assert_eq!(upgraded.sync_pairs[1], f.settings.sync_pairs[1]);
        let repeated = detection::refresh_processing_config(&f.controller, &sidecar, a)
            .await
            .unwrap();
        assert_eq!(repeated, upgraded);
        assert_eq!(load_settings(&f.path).unwrap(), upgraded);
        sidecar.shutdown().await;
    });
}

#[test]
fn preview_timeout_and_aborted_future_release_all_locks_without_saving() {
    runtime().block_on(async {
        for abort in [false, true] {
            let f = Fixture::new();
            let sidecar = f.sidecar();
            let a = f.settings.sync_pairs[0].id;
            let saved =
                detection::save_processing_config(&f.controller, &sidecar, a, rules("Anna"))
                    .await
                    .unwrap();
            let bytes = fs::read(&f.path).unwrap();
            let controller = f.controller.clone();
            let child = sidecar.clone();
            let worker = tokio::spawn(async move {
                detection::preview_rules(&controller, &child, a, rules("HANG"), "Anna".into()).await
            });
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(
                f.controller.try_operation().err().unwrap().code,
                "operation_busy"
            );
            assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_err());
            assert!(CollectionGuard::acquire(&saved.sync_pairs[0].source_folder).is_err());
            if abort {
                worker.abort();
                assert!(worker.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(worker.await.unwrap().unwrap_err().code, "engine_timeout");
            }
            assert_eq!(fs::read(&f.path).unwrap(), bytes);
            assert!(f.controller.try_operation().is_ok());
            let result =
                detection::preview_rules(&f.controller, &sidecar, a, rules("Anna"), "Anna".into())
                    .await
                    .unwrap();
            assert_eq!(result.len(), 1);
            sidecar.shutdown().await;
        }
    });
}
