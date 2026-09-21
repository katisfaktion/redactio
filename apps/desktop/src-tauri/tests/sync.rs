mod common;

use redactio_lib::domain::sync::RunCounts;
use redactio_lib::{
    domain::{
        mapping::{CollectionGuard, Mapping},
        settings::{save_settings, Settings},
        sync::{RunController, RunOutcome},
    },
    sidecar::Sidecar,
};
use std::{fs, path::PathBuf};

struct Fixture {
    _root: tempfile::TempDir,
    controller: RunController,
    settings: Settings,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        let mut settings = Settings::default();
        for label in ["CANARY_PAIR_A", "CANARY_PAIR_B"] {
            let source = root.path().join(format!("{label}_source"));
            let target = root.path().join(format!("{label}_target"));
            fs::create_dir(&source).unwrap();
            fs::create_dir(&target).unwrap();
            settings.add(label, &source, &target).unwrap();
        }
        let model_name = common::fixture_model_store(root.path());
        for pair in &mut settings.sync_pairs {
            pair.config.model = model_name.clone();
        }
        let path = config.join("settings.json");
        save_settings(&path, &settings).unwrap();
        let controller = RunController::new(path.clone());
        Self {
            _root: root,
            controller,
            settings,
            path,
        }
    }
    fn sidecar(&self, mode: &str) -> Sidecar {
        Sidecar::new(
            std::env::var_os("REDACTIO_TEST_PYTHON").unwrap().into(),
            vec![
                common::manifest_dir()
                    .join("tests/fake_sidecar.py")
                    .into_os_string(),
                mode.into(),
            ],
            self._root.path().into(),
        )
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn unreadable_existing_outputs_stop_after_first_storage_failure() {
    use std::os::unix::fs::PermissionsExt;
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        for name in ["a.docx", "b.docx"] {
            fs::write(pair.source_folder.join(name), b"synthetic").unwrap();
        }
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let initial = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(initial.outcome, RunOutcome::Completed);
        let mapping_before =
            Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
        let outputs: Vec<_> = ["doc-0001.md", "doc-0002.md"]
            .into_iter()
            .map(|name| {
                let path = pair.target_folder.join(name);
                let bytes = fs::read(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
                (path, bytes)
            })
            .collect();

        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |progress| {
                assert!(progress.counts.is_consistent());
            })
            .await;
        for (path, bytes) in outputs {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(summary.outcome, RunOutcome::Failed);
        assert_eq!(
            summary.progress.counts,
            RunCounts {
                discovered: 2,
                failed: 1,
                unprocessed: 1,
                ..RunCounts::default()
            }
        );
        assert_eq!(summary.errors.len(), 1);
        assert_eq!(summary.errors[0].relative_path, "a.docx");
        assert_eq!(summary.errors[0].error.code, "permission_denied");
        assert_eq!(
            Mapping::load(&pair.source_folder, pair.id, &pair.target_folder)
                .unwrap()
                .entries(),
            mapping_before.entries()
        );
        assert!(f.controller.prepare(pair.id, None, vec![]).is_ok());
    });
}

#[cfg(unix)]
#[test]
fn unreadable_source_does_not_prevent_processing_readable_peer() {
    use std::os::unix::fs::PermissionsExt;
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        let unreadable = pair.source_folder.join("a.docx");
        fs::write(&unreadable, b"synthetic").unwrap();
        fs::write(pair.source_folder.join("b.docx"), b"synthetic peer").unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(summary.outcome, RunOutcome::CompletedWithErrors);
        assert_eq!(
            summary.progress.counts,
            RunCounts {
                discovered: 2,
                processed: 1,
                failed: 1,
                ..RunCounts::default()
            }
        );
        assert_eq!(summary.errors[0].relative_path, "a.docx");
        assert_eq!(summary.errors[0].error.code, "permission_denied");
        assert!(pair.target_folder.join("doc-0001.md").is_file());
    });
}

#[cfg(unix)]
#[test]
fn output_io_failure_does_not_publish_a_success_and_releases_the_operation() {
    use std::os::unix::fs::PermissionsExt;
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        fs::write(pair.source_folder.join("a.docx"), b"synthetic").unwrap();
        fs::write(pair.source_folder.join("b.docx"), b"synthetic peer").unwrap();
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |progress| {
                if progress.stage == redactio_lib::domain::sync::RunStage::Processing {
                    fs::set_permissions(&pair.target_folder, fs::Permissions::from_mode(0o500))
                        .unwrap();
                }
            })
            .await;
        fs::set_permissions(&pair.target_folder, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(summary.progress.counts.processed, 0);
        assert_eq!(summary.progress.counts.failed, 1);
        assert_eq!(summary.progress.counts.unprocessed, 1);
        assert_eq!(summary.outcome, RunOutcome::Failed);
        assert!(!pair.target_folder.join("doc-0001.md").exists());
        assert!(
            Mapping::load(&pair.source_folder, pair.id, &pair.target_folder)
                .unwrap()
                .entries()
                .iter()
                .all(|entry| entry.committed.is_none())
        );
        assert!(f.controller.prepare(pair.id, None, vec![]).is_ok());
    });
}

#[test]
#[ignore = "requires the reviewed sidecar interpreter and bundled models"]
fn real_two_pair_batches_cover_errors_cancellation_and_rule_isolation() {
    use redactio_lib::domain::settings::{CustomRule, EntityType};
    runtime().block_on(async {
        let mut f = Fixture::new();
        let python = std::env::var_os("REDACTIO_TEST_PYTHON").unwrap();
        for (index, term) in ["CanaryAlpha", "CanaryBeta"].iter().enumerate() {
            let pair = &mut f.settings.sync_pairs[index];
            pair.config.enabled_entities.clear();
            pair.config.custom_rules.push(CustomRule::Words { id: uuid::Uuid::new_v4(), entity_type: EntityType::Custom, enabled: true, words: vec![(*term).into()] });
            for name in ["a.docx", "b.docx"] {
                let status = std::process::Command::new(&python).args(["-c", "from docx import Document; import sys; d=Document(); d.add_paragraph('CanaryAlpha und CanaryBeta.'); d.save(sys.argv[1])"]).arg(pair.source_folder.join(name)).status().unwrap();
                assert!(status.success());
            }
        }
        fs::write(f.settings.sync_pairs[0].source_folder.join("b.docx"), b"corrupt zip").unwrap();
        save_settings(&f.path, &f.settings).unwrap();
        let sidecar = Sidecar::new(python.into(), vec!["-m".into(), "redactio_sidecar".into()], std::env::var_os("REDACTIO_MODEL_DIR").unwrap().into());
        let a = &f.settings.sync_pairs[0]; let b = &f.settings.sync_pairs[1];
        let run = f.controller.prepare(a.id, None, vec![]).unwrap();
        let first = f.controller.execute(run, Ok(sidecar.clone()), |_| {}).await;
        assert_eq!(first.outcome, RunOutcome::CompletedWithErrors, "run error: {:?}, file errors: {:?}", first.error, first.errors); assert_eq!(first.progress.counts.processed, 1); assert_eq!(first.progress.counts.failed, 1);
        let output_a = fs::read(a.target_folder.join("doc-0001.md")).unwrap();
        let text_a = String::from_utf8(output_a.clone()).unwrap(); assert!(!text_a.contains("CanaryAlpha")); assert!(text_a.contains("CanaryBeta"));
        let run = f.controller.prepare(b.id, None, vec![]).unwrap(); let id = run.run_id;
        let cancelled = f.controller.execute(run, Ok(sidecar.clone()), |progress| {
            if progress.stage == redactio_lib::domain::sync::RunStage::Processing && progress.counts.processed == 1 { f.controller.request_cancel(b.id, id).unwrap(); }
        }).await;
        assert_eq!(cancelled.outcome, RunOutcome::Cancelled); assert_eq!(cancelled.progress.counts.unprocessed, 1);
        let text_b = fs::read_to_string(b.target_folder.join("doc-0001.md")).unwrap(); assert!(text_b.contains("CanaryAlpha")); assert!(!text_b.contains("CanaryBeta"));
        assert_eq!(fs::read(a.target_folder.join("doc-0001.md")).unwrap(), output_a);
        let run = f.controller.prepare(a.id, Some(vec!["a.docx".into()]), vec![]).unwrap();
        let repeat = f.controller.execute(run, Ok(sidecar.clone()), |_| {}).await; assert_eq!(repeat.progress.counts.skipped, 1);
        assert_eq!(fs::read(a.target_folder.join("doc-0001.md")).unwrap(), output_a);
        let audit = fs::read_to_string(f.path.parent().unwrap().join("audit-log.jsonl")).unwrap();
        assert!(!audit.contains("CanaryAlpha")); assert!(!audit.contains("CanaryBeta")); assert!(!audit.contains("CANARY"));
        sidecar.shutdown().await;
    });
}

#[test]
fn cancelled_after_first_commit_preserves_result_releases_guard_and_isolates_pairs() {
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        for name in ["a.docx", "b.docx"] {
            fs::write(pair.source_folder.join(name), b"synthetic").unwrap();
        }
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let id = run.run_id;
        let controller = f.controller.clone();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |progress| {
                assert!(progress.counts.is_consistent());
                if progress.counts.processed == 1
                    && progress.stage == redactio_lib::domain::sync::RunStage::Processing
                {
                    controller.request_cancel(pair.id, id).unwrap();
                }
            })
            .await;
        assert_eq!(summary.outcome, RunOutcome::Cancelled);
        assert_eq!(
            summary.progress.counts,
            RunCounts {
                discovered: 2,
                processed: 1,
                unprocessed: 1,
                ..RunCounts::default()
            }
        );
        assert!(pair.target_folder.join("doc-0001.md").is_file());
        assert!(!pair.target_folder.join("doc-0002.md").exists());
        assert_eq!(
            fs::read_dir(&f.settings.sync_pairs[1].target_folder)
                .unwrap()
                .count(),
            0
        );
        CollectionGuard::acquire(&pair.source_folder).unwrap();
        assert_eq!(
            f.controller.summary(pair.id, id).unwrap().unwrap().outcome,
            RunOutcome::Cancelled
        );
        let next = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let complete = f
            .controller
            .execute(next, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(complete.outcome, RunOutcome::Completed);
        assert_eq!(complete.progress.counts.skipped, 1);
        assert_eq!(complete.progress.counts.processed, 1);
    });
}

#[test]
fn empty_startup_failure_and_duplicate_attempts_leave_audited_summaries() {
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        assert!(f.controller.prepare(pair.id, None, vec![]).is_err());
        assert!(f
            .controller
            .request_cancel(f.settings.sync_pairs[1].id, run.run_id)
            .is_err());
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(summary.progress.counts, RunCounts::default());
        assert_eq!(summary.outcome, RunOutcome::Completed);
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let missing = Sidecar::new(
            f._root.path().join("missing"),
            vec![],
            f._root.path().into(),
        );
        let failed = f.controller.execute(run, Ok(missing), |_| {}).await;
        assert_eq!(failed.outcome, RunOutcome::Failed);
        assert!(!failed.audit_warning);
        let audit = fs::read_to_string(f.path.parent().unwrap().join("audit-log.jsonl")).unwrap();
        assert_eq!(audit.lines().count(), 2);
        assert!(!audit.contains("CANARY"));
    });
}

#[test]
fn individual_failures_continue_warnings_are_subsets_and_retry_paths_are_allowlisted() {
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        for (name, body) in [
            ("a.docx", "invalid"),
            ("b.docx", "warning"),
            ("c.docx", "synthetic"),
        ] {
            fs::write(pair.source_folder.join(name), body).unwrap();
        }
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(summary.outcome, RunOutcome::CompletedWithErrors);
        assert_eq!(
            summary.progress.counts,
            RunCounts {
                discovered: 3,
                processed: 2,
                failed: 1,
                warned: 1,
                ..RunCounts::default()
            }
        );
        assert_eq!(summary.errors[0].relative_path, "a.docx");
        let mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
        assert!(mapping.entries()[0].committed.is_none());
        let run = f
            .controller
            .prepare(pair.id, Some(vec!["../outside.docx".into()]), vec![])
            .unwrap();
        let rejected = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(rejected.outcome, RunOutcome::Failed);
        assert_eq!(rejected.error.unwrap().code, "invalid_selection");
    });
}

#[test]
fn cancellation_during_initialization_has_a_final_summary_and_releases_both_locks() {
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let id = run.run_id;
        let controller = f.controller.clone();
        let sidecar = f.sidecar("batch-slow-init");
        let task = tokio::spawn(async move { controller.execute(run, Ok(sidecar), |_| {}).await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        f.controller.cancel(pair.id, id).await.unwrap();
        let summary = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.outcome, RunOutcome::Cancelled);
        assert!(!summary.audit_warning);
        CollectionGuard::acquire(f.path.parent().unwrap()).unwrap();
        CollectionGuard::acquire(&pair.source_folder).unwrap();
        assert!(f.controller.prepare(pair.id, None, vec![]).is_ok());
    });
}

#[test]
fn audit_failure_preserves_output_and_canaries_stay_private() {
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        fs::write(
            pair.source_folder.join("CANARY_FILENAME.docx"),
            b"CANARY_SOURCE",
        )
        .unwrap();
        fs::create_dir(f.path.parent().unwrap().join("audit-log.jsonl")).unwrap();
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let mut events = Vec::new();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |progress| {
                events.push(serde_json::to_string(progress).unwrap())
            })
            .await;
        assert_eq!(summary.outcome, RunOutcome::Completed);
        assert!(summary.audit_warning);
        let output = fs::read_to_string(pair.target_folder.join("doc-0001.md")).unwrap();
        assert!(!output.contains("CANARY"));
        assert!(events.iter().all(|event| !event.contains("CANARY")));
        let mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
        assert!(mapping.entries()[0].committed.is_some());
    });
}

#[test]
fn source_changes_are_rejected_before_publication_and_later_files_continue() {
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        fs::write(pair.source_folder.join("a.docx"), b"change").unwrap();
        fs::write(pair.source_folder.join("b.docx"), b"synthetic").unwrap();
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        let summary = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(summary.outcome, RunOutcome::CompletedWithErrors);
        assert_eq!(summary.errors[0].error.code, "source_changed");
        assert!(!pair.target_folder.join("doc-0001.md").exists());
        assert!(pair.target_folder.join("doc-0002.md").exists());
    });
}

#[test]
fn a_current_scan_entry_is_rechecked_before_it_can_be_skipped() {
    runtime().block_on(async {
        for source_changed in [false, true] {
            let f = Fixture::new();
            let pair = &f.settings.sync_pairs[0];
            fs::write(pair.source_folder.join("a.docx"), b"synthetic").unwrap();
            let first = f.controller.prepare(pair.id, None, vec![]).unwrap();
            f.controller
                .execute(first, Ok(f.sidecar("batch")), |_| {})
                .await;
            let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
            let summary = f
                .controller
                .execute(run, Ok(f.sidecar("batch")), |progress| {
                    if progress.stage == redactio_lib::domain::sync::RunStage::Processing
                        && progress.counts.completed() == 0
                    {
                        let path = if source_changed {
                            pair.source_folder.join("a.docx")
                        } else {
                            pair.target_folder.join("doc-0001.md")
                        };
                        fs::write(path, b"changed after scan").unwrap();
                    }
                })
                .await;
            assert_eq!(summary.progress.counts.skipped, 0);
            assert_eq!(summary.progress.counts.failed, 1);
            assert_eq!(
                summary.errors[0].error.code,
                if source_changed {
                    "source_changed"
                } else {
                    "output_conflict"
                }
            );
        }
    });
}

#[test]
fn reviewed_work_requires_explicit_force_and_force_never_overwrites_external_edits() {
    runtime().block_on(async {
        use redactio_lib::{
            domain::mapping::{commit_generation, CommitCandidate, ReviewRecord},
            protocol::DocumentKey,
        };
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        fs::write(pair.source_folder.join("a.docx"), b"synthetic").unwrap();
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        f.controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        let settings = redactio_lib::domain::settings::load_settings(&f.path).unwrap();
        let pair = &settings.sync_pairs[0];
        {
            let config_guard = CollectionGuard::acquire(f.path.parent().unwrap()).unwrap();
            let guard = CollectionGuard::acquire(&pair.source_folder).unwrap();
            let mut mapping =
                Mapping::load(&pair.source_folder, pair.id, &pair.target_folder).unwrap();
            let generation = mapping.entries()[0].committed.clone().unwrap();
            let key = DocumentKey {
                sync_pair_id: pair.id,
                doc_id: "doc-0001".into(),
            };
            let mut review = ReviewRecord::read(&pair.source_folder, &key, &generation).unwrap();
            review.notes = "CANARY_PRIVATE_NOTE".into();
            let candidate = CommitCandidate {
                key,
                source_hash: review.source_hash.clone(),
                revision: pair.processing_revision,
                markdown: fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
                review,
            };
            commit_generation(
                pair,
                &mut mapping,
                candidate,
                Some(&generation.output_hash),
                &guard,
                &f.path,
                &config_guard,
            )
            .unwrap();
        }
        let mut changed_config = pair.config.clone();
        changed_config.include_positions = false;
        redactio_lib::domain::detection::save_processing_config(
            &f.controller,
            &f.sidecar("batch"),
            pair.id,
            changed_config,
        )
        .await
        .unwrap();
        let reviewed_output = fs::read(pair.target_folder.join("doc-0001.md")).unwrap();
        let run = f
            .controller
            .prepare(pair.id, Some(vec!["a.docx".into()]), vec![])
            .unwrap();
        let blocked = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(blocked.errors[0].error.code, "confirmation_required");
        assert_eq!(
            fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
            reviewed_output
        );
        let run = f
            .controller
            .prepare(
                pair.id,
                Some(vec!["a.docx".into()]),
                vec!["doc-0001".into()],
            )
            .unwrap();
        let confirmed = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(confirmed.outcome, RunOutcome::Completed);
        fs::write(pair.target_folder.join("doc-0001.md"), b"EXTERNAL_EDIT").unwrap();
        let run = f
            .controller
            .prepare(pair.id, None, vec!["doc-0001".into()])
            .unwrap();
        let conflict = f
            .controller
            .execute(run, Ok(f.sidecar("batch")), |_| {})
            .await;
        assert_eq!(conflict.errors[0].error.code, "output_conflict");
        assert_eq!(
            fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
            b"EXTERNAL_EDIT"
        );
    });
}

#[test]
fn missing_partial_stage_is_rebuilt_only_from_the_exact_journal_candidate() {
    runtime().block_on(async {
        use redactio_lib::{
            domain::mapping::{MappingData, PendingCommit, ReviewRecord},
            protocol::DocumentKey,
        };
        for changed in [false, true] {
            let f = Fixture::new();
            let pair = &f.settings.sync_pairs[0];
            fs::write(pair.source_folder.join("a.docx"), b"synthetic").unwrap();
            let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
            f.controller
                .execute(run, Ok(f.sidecar("batch")), |_| {})
                .await;
            let mut mapping = MappingData::read(&pair.source_folder).unwrap();
            let generation = mapping.entries[0].committed.take().unwrap();
            let key = DocumentKey {
                sync_pair_id: pair.id,
                doc_id: "doc-0001".into(),
            };
            let review = ReviewRecord::read(&pair.source_folder, &key, &generation).unwrap();
            mapping.entries[0].pending = Some(PendingCommit {
                generation: generation.clone(),
                prior_output_hash: None,
                prior_review_hash: None,
                review,
                output_temporary: format!(".redactio-commit-{}.tmp", uuid::Uuid::new_v4()),
                review_temporary: format!(".redactio-commit-{}.tmp", uuid::Uuid::new_v4()),
            });
            fs::write(
                pair.source_folder.join("_document-mapping.json"),
                serde_json::to_vec_pretty(&mapping).unwrap(),
            )
            .unwrap();
            fs::remove_file(pair.source_folder.join("_redactio/reviews/doc-0001.json")).unwrap();
            if changed {
                fs::write(pair.source_folder.join("a.docx"), b"changed after crash").unwrap();
            }
            let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
            let summary = f
                .controller
                .execute(run, Ok(f.sidecar("batch")), |_| {})
                .await;
            let after = MappingData::read(&pair.source_folder).unwrap();
            if changed {
                assert_eq!(summary.errors[0].error.code, "recovery_pending");
                assert!(after.entries[0].pending.is_some());
            } else {
                assert_eq!(summary.outcome, RunOutcome::Completed);
                assert!(after.entries[0].pending.is_none());
                assert_eq!(after.entries[0].committed.as_ref().unwrap(), &generation);
            }
        }
    });
}

#[test]
fn configuration_changes_reconcile_old_journals_without_stranding_incomplete_candidates() {
    runtime().block_on(async {
        use redactio_lib::{
            domain::{
                detection,
                mapping::{MappingData, PendingCommit, ReviewRecord},
                settings::load_settings,
            },
            protocol::DocumentKey,
        };
        use uuid::Uuid;
        for staged in [false, true] {
            let f = Fixture::new();
            let id = f.settings.sync_pairs[0].id;
            fs::write(
                f.settings.sync_pairs[0].source_folder.join("a.docx"),
                b"synthetic",
            )
            .unwrap();
            let run = f.controller.prepare(id, None, vec![]).unwrap();
            assert_eq!(
                f.controller
                    .execute(run, Ok(f.sidecar("batch")), |_| {})
                    .await
                    .outcome,
                RunOutcome::Completed
            );
            let saved = load_settings(&f.path).unwrap();
            let pair = &saved.sync_pairs[0];
            let mut mapping = MappingData::read(&pair.source_folder).unwrap();
            let generation = mapping.entries[0].committed.take().unwrap();
            let review_path = pair.source_folder.join("_redactio/reviews/doc-0001.json");
            let review_bytes = fs::read(&review_path).unwrap();
            let review = ReviewRecord::read(
                &pair.source_folder,
                &DocumentKey {
                    sync_pair_id: id,
                    doc_id: "doc-0001".into(),
                },
                &generation,
            )
            .unwrap();
            let pending = PendingCommit {
                generation: generation.clone(),
                prior_output_hash: None,
                prior_review_hash: None,
                review,
                output_temporary: format!(".redactio-commit-{}.tmp", Uuid::new_v4()),
                review_temporary: format!(".redactio-commit-{}.tmp", Uuid::new_v4()),
            };
            if staged {
                fs::write(
                    review_path
                        .parent()
                        .unwrap()
                        .join(&pending.review_temporary),
                    &review_bytes,
                )
                .unwrap();
            }
            mapping.entries[0].pending = Some(pending.clone());
            fs::write(
                pair.source_folder.join("_document-mapping.json"),
                serde_json::to_vec_pretty(&mapping).unwrap(),
            )
            .unwrap();
            fs::remove_file(&review_path).unwrap();
            let bytes = fs::read(&f.path).unwrap();
            let output = fs::read(pair.target_folder.join("doc-0001.md")).unwrap();
            let mut config = pair.config.clone();
            config.include_positions = false;
            let result =
                detection::save_processing_config(&f.controller, &f.sidecar("batch"), id, config)
                    .await;
            let after = MappingData::read(&pair.source_folder).unwrap();
            if staged {
                assert_ne!(
                    result.unwrap().sync_pairs[0].processing_revision,
                    pair.processing_revision
                );
                assert!(after.entries[0].pending.is_none());
                assert_eq!(after.entries[0].committed.as_ref().unwrap(), &generation);
                assert_eq!(fs::read(&review_path).unwrap(), review_bytes);
            } else {
                assert_eq!(result.unwrap_err().code, "recovery_pending");
                assert_eq!(fs::read(&f.path).unwrap(), bytes);
                assert_eq!(after.entries[0].pending.as_ref(), Some(&pending));
                let run = f.controller.prepare(id, None, vec![]).unwrap();
                assert_eq!(
                    f.controller
                        .execute(run, Ok(f.sidecar("batch")), |_| {})
                        .await
                        .outcome,
                    RunOutcome::Completed
                );
                assert_eq!(fs::read(&review_path).unwrap(), review_bytes);
            }
            assert_eq!(
                fs::read(pair.target_folder.join("doc-0001.md")).unwrap(),
                output
            );
        }
    });
}

#[test]
fn skipped_and_warned_do_not_inflate_the_denominator() {
    let counts = RunCounts {
        discovered: 4,
        processed: 1,
        skipped: 1,
        failed: 1,
        unprocessed: 1,
        warned: 1,
    };
    assert!(counts.is_consistent());
    assert_eq!(counts.completed(), 3);
}

#[test]
fn counts_reject_missing_documents_and_warnings_without_processing() {
    assert!(RunCounts::default().is_consistent());
    assert!(!RunCounts {
        discovered: 1,
        ..RunCounts::default()
    }
    .is_consistent());
    assert!(!RunCounts {
        warned: 1,
        ..RunCounts::default()
    }
    .is_consistent());
    assert!(RunCounts {
        discovered: 1,
        unprocessed: 1,
        ..RunCounts::default()
    }
    .is_consistent());
}

#[test]
fn overflow_never_wraps_into_a_consistent_count() {
    let largest = RunCounts {
        discovered: u64::MAX,
        processed: u64::MAX,
        warned: u64::MAX,
        ..RunCounts::default()
    };
    assert!(largest.is_consistent());
    assert_eq!(largest.completed(), u64::MAX);
    for counts in [
        RunCounts {
            skipped: 1,
            ..largest
        },
        RunCounts {
            failed: 1,
            ..largest
        },
        RunCounts {
            unprocessed: 1,
            ..largest
        },
        RunCounts {
            discovered: 0,
            skipped: 1,
            ..largest
        },
    ] {
        assert!(!counts.is_consistent());
        assert_eq!(counts.completed(), u64::MAX);
    }
}

#[test]
fn registered_missing_root_preparation_is_an_audited_failed_run_with_visible_audit_warning() {
    runtime().block_on(async {
        for target_missing in [false, true] {
            for audit_blocked in [false, true] {
                let f = Fixture::new();
                let pair = &f.settings.sync_pairs[0];
                let unavailable = if target_missing {
                    &pair.target_folder
                } else {
                    &pair.source_folder
                };
                let retained = f._root.path().join("retained-root");
                fs::rename(unavailable, &retained).unwrap();
                let before: Vec<_> = fs::read_dir(&retained)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .collect();
                let audit = f.path.parent().unwrap().join("audit-log.jsonl");
                if audit_blocked {
                    fs::create_dir(&audit).unwrap();
                }
                let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
                let run_id = run.run_id;
                assert!(f.controller.try_operation().is_err());
                assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_err());
                let mut finished = false;
                let summary = f
                    .controller
                    .execute(
                        run,
                        Err(std::io::Error::other("CANARY_SIDECAR_FAILURE").into()),
                        |progress| {
                            if progress.stage == redactio_lib::domain::sync::RunStage::Finished {
                                finished = true;
                            }
                        },
                    )
                    .await;
                assert!(finished);
                assert_eq!(summary.outcome, RunOutcome::Failed);
                assert_eq!(summary.error.as_ref().unwrap().code, "path_unavailable");
                assert_eq!(summary.audit_warning, audit_blocked);
                assert_eq!(summary.progress.counts, RunCounts::default());
                assert_eq!(
                    f.controller
                        .summary(pair.id, run_id)
                        .unwrap()
                        .unwrap()
                        .audit_warning,
                    audit_blocked
                );
                assert!(!unavailable.exists());
                assert_eq!(
                    fs::read_dir(&retained)
                        .unwrap()
                        .map(|entry| entry.unwrap().file_name())
                        .collect::<Vec<_>>(),
                    before
                );
                if !audit_blocked {
                    let bytes = fs::read_to_string(&audit).unwrap();
                    let entry: serde_json::Value = serde_json::from_str(bytes.trim()).unwrap();
                    assert_eq!(entry["sync_pair_id"], pair.id.to_string());
                    assert_eq!(entry["action"], "sync");
                    assert_eq!(entry["outcome"], "failed");
                    assert_eq!(
                        entry["error_codes"],
                        serde_json::json!(["path_unavailable"])
                    );
                    assert!(entry["engine"].is_null());
                    assert!(!bytes.contains("CANARY"));
                }
                assert!(f.controller.try_operation().is_ok());
                assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_ok());
            }
        }
    });
}

#[test]
fn preparation_keeps_eager_source_exclusion_and_busy_unknown_attempts_unadmitted() {
    let f = Fixture::new();
    let pair = &f.settings.sync_pairs[0];
    let audit = f.path.parent().unwrap().join("audit-log.jsonl");
    let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
    assert!(CollectionGuard::acquire(&pair.source_folder).is_err());
    assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_err());
    assert_eq!(
        f.controller
            .prepare(pair.id, None, vec![])
            .err()
            .unwrap()
            .code,
        "operation_busy"
    );
    drop(run);
    let unknown = uuid::Uuid::new_v4();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let error = f.controller.prepare(unknown, None, vec![]).err().unwrap();
        // Concurrent test sidecars briefly inherit a dropped lock until exec.
        // Configuration locking correctly precedes unknown-pair validation.
        if error.code != "file_busy" || std::time::Instant::now() >= deadline {
            assert_eq!(error.code, "unknown_pair");
            break;
        }
        std::thread::yield_now();
    }
    let held = CollectionGuard::acquire(&pair.source_folder).unwrap();
    assert_eq!(
        f.controller
            .prepare(pair.id, None, vec![])
            .err()
            .unwrap()
            .code,
        "file_busy"
    );
    assert!(f.controller.try_operation().is_ok());
    assert!(CollectionGuard::acquire(f.path.parent().unwrap()).is_ok());
    assert!(!audit.exists());
    drop(held);
    assert!(f.controller.prepare(pair.id, None, vec![]).is_ok());
}

fn review_input(
    view: &redactio_lib::domain::review::ReviewViewData,
    status: &str,
) -> redactio_lib::domain::review::SaveReview {
    redactio_lib::domain::review::SaveReview {
        expected_output_hash: view.expected_output_hash.clone(),
        expected_review_hash: view.expected_review_hash.clone(),
        decisions: view.decisions.clone(),
        status: serde_json::from_value(status.into()).unwrap(),
        notes: String::new(),
        acknowledged_warnings: vec![],
    }
}

#[test]
fn explicit_empty_rework_save_is_protected_but_automatic_warning_is_not() {
    use redactio_lib::domain::review::{open_configured, save_configured};
    use redactio_lib::protocol::DocumentKey;
    runtime().block_on(async {
        for explicit in [false, true] {
            let f = Fixture::new();
            let pair = &f.settings.sync_pairs[0];
            fs::write(
                pair.source_folder.join("a.docx"),
                if explicit {
                    b"synthetic".as_slice()
                } else {
                    b"warning".as_slice()
                },
            )
            .unwrap();
            let sidecar = f.sidecar("batch");
            let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
            assert_eq!(
                f.controller
                    .execute(run, Ok(sidecar.clone()), |_| {})
                    .await
                    .progress
                    .counts
                    .processed,
                1
            );
            let key = DocumentKey {
                sync_pair_id: pair.id,
                doc_id: "doc-0001".into(),
            };
            if explicit {
                let view = open_configured(&f.controller, &sidecar, &key)
                    .await
                    .unwrap();
                save_configured(
                    &f.controller,
                    &sidecar,
                    &key,
                    review_input(&view, "needs-rework"),
                )
                .await
                .unwrap();
            }
            let private_path = pair.source_folder.join("_redactio/reviews/doc-0001.json");
            let before = fs::read(&private_path).unwrap();
            fs::write(pair.source_folder.join("a.docx"), b"new source").unwrap();
            let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
            let result = f.controller.execute(run, Ok(sidecar.clone()), |_| {}).await;
            if explicit {
                assert_eq!(
                    result.errors.first().map(|e| e.error.code.as_str()),
                    Some("confirmation_required")
                );
                assert_eq!(fs::read(&private_path).unwrap(), before);
                let run = f
                    .controller
                    .prepare(pair.id, None, vec![key.doc_id])
                    .unwrap();
                assert_eq!(
                    f.controller
                        .execute(run, Ok(sidecar.clone()), |_| {})
                        .await
                        .progress
                        .counts
                        .processed,
                    1
                );
            } else {
                assert_eq!(result.progress.counts.processed, 1);
            }
            sidecar.shutdown().await;
        }
    });
}

#[test]
fn private_generation_rejects_stale_notes_and_warning_acknowledgement() {
    use redactio_lib::domain::review::{open_configured, save_configured};
    use redactio_lib::protocol::DocumentKey;
    runtime().block_on(async {
        for warning in [false, true] {
            let f = Fixture::new();
            let pair = &f.settings.sync_pairs[0];
            fs::write(
                pair.source_folder.join("a.docx"),
                if warning {
                    b"warning".as_slice()
                } else {
                    b"synthetic".as_slice()
                },
            )
            .unwrap();
            let sidecar = f.sidecar("batch");
            let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
            f.controller.execute(run, Ok(sidecar.clone()), |_| {}).await;
            let key = DocumentKey {
                sync_pair_id: pair.id,
                doc_id: "doc-0001".into(),
            };
            let a = open_configured(&f.controller, &sidecar, &key)
                .await
                .unwrap();
            let b = open_configured(&f.controller, &sidecar, &key)
                .await
                .unwrap();
            let status = if warning { "needs-rework" } else { "pending" };
            let mut input = review_input(&b, status);
            if warning {
                input.acknowledged_warnings = b.warnings.clone();
            } else {
                input.notes = "PRIVATE_B".into();
            }
            let saved = save_configured(&f.controller, &sidecar, &key, input)
                .await
                .unwrap();
            assert_eq!(
                saved.expected_output_hash, a.expected_output_hash,
                "fixture isolates private generation"
            );
            assert!(!saved.markdown.contains("PRIVATE_B"));
            let path = pair.source_folder.join("_redactio/reviews/doc-0001.json");
            let before = fs::read(&path).unwrap();
            let result =
                save_configured(&f.controller, &sidecar, &key, review_input(&a, status)).await;
            assert_eq!(result.unwrap_err().code, "review_conflict");
            assert_eq!(fs::read(path).unwrap(), before);
            sidecar.shutdown().await;
        }
    });
}

#[test]
fn scan_projects_only_valid_current_private_review_states() {
    use redactio_lib::{
        domain::{
            review::{open_configured, save_configured},
            scan::scan_collection,
        },
        protocol::DocumentKey,
    };
    runtime().block_on(async {
        let f = Fixture::new();
        let pair = &f.settings.sync_pairs[0];
        let statuses = [
            "pending",
            "approved",
            "rejected",
            "needs-rework",
            "approved",
            "approved",
        ];
        for index in 0..statuses.len() {
            fs::write(
                pair.source_folder.join(format!("{index}.docx")),
                b"synthetic",
            )
            .unwrap();
        }
        let sidecar = f.sidecar("batch");
        let run = f.controller.prepare(pair.id, None, vec![]).unwrap();
        f.controller.execute(run, Ok(sidecar.clone()), |_| {}).await;
        for (index, status) in statuses.iter().enumerate() {
            let key = DocumentKey {
                sync_pair_id: pair.id,
                doc_id: format!("doc-{:04}", index + 1),
            };
            let view = open_configured(&f.controller, &sidecar, &key)
                .await
                .unwrap();
            save_configured(&f.controller, &sidecar, &key, review_input(&view, status))
                .await
                .unwrap();
        }
        fs::write(pair.source_folder.join("4.docx"), b"changed").unwrap();
        fs::write(
            pair.source_folder.join("_redactio/reviews/doc-0006.json"),
            b"{}",
        )
        .unwrap();
        let settings = redactio_lib::domain::settings::load_settings(&f.path).unwrap();
        let report =
            serde_json::to_value(scan_collection(&settings.sync_pairs[0]).unwrap()).unwrap();
        for (index, status) in statuses[..4].iter().enumerate() {
            assert_eq!(report["files"][index]["review_status"], *status);
        }
        assert_eq!(report["files"][4]["state"], "stale");
        assert!(report["files"][4]["review_status"].is_null());
        assert_eq!(report["files"][5]["state"], "conflict");
        assert!(report["files"][5]["review_status"].is_null());
        assert_eq!(report["errors"][0]["code"], "review_mismatch");
        sidecar.shutdown().await;
    });
}
