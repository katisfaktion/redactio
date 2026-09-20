mod common;

#[cfg(debug_assertions)]
use redactio_lib::resources;
use redactio_lib::{
    domain::settings::ProcessingConfig,
    protocol::{
        ConfigurePayload, ConfigureResult, Detection, OutputEntry, PingResult, PreviewRulesPayload,
        PreviewRulesResult, ProcessRequest, ProcessResult, ReviewRequest,
    },
    sidecar::Sidecar,
};
use std::{
    ffi::OsString,
    io::Write as _,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;

fn fake(mode: &str) -> Sidecar {
    let script = common::manifest_dir().join("tests/fake_sidecar.py");
    let python = std::env::var_os("REDACTIO_TEST_PYTHON")
        .expect("Set REDACTIO_TEST_PYTHON to the test interpreter's absolute path");
    Sidecar::new(
        PathBuf::from(python),
        vec![script.into_os_string(), OsString::from(mode)],
        PathBuf::from("."),
    )
}

fn fake_with_marker(mode: &str, marker: &std::path::Path) -> Sidecar {
    let script = common::manifest_dir().join("tests/fake_sidecar.py");
    let python = std::env::var_os("REDACTIO_TEST_PYTHON")
        .expect("Set REDACTIO_TEST_PYTHON to the test interpreter's absolute path");
    Sidecar::new(
        PathBuf::from(python),
        vec![
            script.into_os_string(),
            OsString::from(mode),
            marker.as_os_str().to_owned(),
        ],
        PathBuf::from("."),
    )
}

fn configuration(pair: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "sync_pair_id": pair,
        "processing_revision": revision,
        "config": {
            "model": "de_core_news_sm",
            "enabled_entities": ["PERSON", "EMAIL_ADDRESS"],
            "custom_rules": [],
            "include_positions": false
        }
    })
}

fn document(pair: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "sync_pair_id": pair,
        "processing_revision": revision,
        "source_path": "synthetic.docx"
    })
}

async fn wait_for_marker(marker: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake child did not reach the expected state");
}

#[test]
fn blocked_stdin_write_obeys_timeout_and_shutdown() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("never-read");
            let child = fake_with_marker("never-read", &marker);
            let started = Instant::now();
            let request_child = child.clone();
            let request = tokio::spawn(async move {
                request_child
                    .request::<_, serde_json::Value>(
                        "ping",
                        &serde_json::json!({"padding": "x".repeat(2 * 1024 * 1024)}),
                        Duration::from_millis(100),
                    )
                    .await
            });
            wait_for_marker(&marker).await;

            let error = tokio::time::timeout(Duration::from_secs(1), request)
                .await
                .expect("blocked write must honor its deadline")
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, "engine_timeout");
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "blocked writer delayed timeout for {:?}",
                started.elapsed()
            );
            tokio::time::timeout(Duration::from_secs(1), child.shutdown())
                .await
                .expect("shutdown must not wait for the blocked writer");
        });
}

#[test]
fn shutdown_cancels_a_blocked_stdin_write() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("never-read-cancel");
            let child = fake_with_marker("never-read", &marker);
            let request_child = child.clone();
            let request = tokio::spawn(async move {
                request_child
                    .request::<_, serde_json::Value>(
                        "ping",
                        &serde_json::json!({"padding": "x".repeat(2 * 1024 * 1024)}),
                        Duration::from_secs(10),
                    )
                    .await
            });
            wait_for_marker(&marker).await;

            tokio::time::timeout(Duration::from_secs(1), child.shutdown())
                .await
                .expect("shutdown must kill independently of the blocked writer");
            let error = tokio::time::timeout(Duration::from_secs(1), request)
                .await
                .expect("cancelled blocked write must finish promptly")
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, "operation_cancelled");
        });
}

#[test]
fn aborting_active_request_kills_its_process_before_unlocking_next_request() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("abort-steal");
            let child = fake_with_marker("abort-steal", &marker);
            let request_child = child.clone();
            let abandoned = tokio::spawn(async move {
                request_child
                    .request::<_, serde_json::Value>(
                        "ping",
                        &serde_json::json!({"abandoned": true}),
                        Duration::from_secs(10),
                    )
                    .await
            });
            wait_for_marker(&marker).await;
            abandoned.abort();
            assert!(abandoned.await.unwrap_err().is_cancelled());

            let response: serde_json::Value = child
                .request(
                    "ping",
                    &serde_json::json!({"current": true}),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
            assert_eq!(response, serde_json::json!({"current": true}));
            child.shutdown().await;
        });
}

#[cfg(unix)]
#[test]
fn dropping_last_owner_kills_and_reaps_an_active_child() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("pid-hang");
            let sidecar = fake_with_marker("pid-hang", &marker);
            let request = tokio::spawn(async move {
                sidecar
                    .request::<_, serde_json::Value>(
                        "ping",
                        &serde_json::json!({}),
                        Duration::from_secs(10),
                    )
                    .await
            });
            wait_for_marker(&marker).await;
            let pid = std::fs::read_to_string(&marker)
                .unwrap()
                .parse::<u32>()
                .unwrap();

            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            tokio::time::timeout(Duration::from_secs(1), async {
                while std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("last-owner drop must kill and reap the child");
        });
}

#[test]
fn hung_child_is_reaped_before_reuse() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("hang");
            let started = Instant::now();

            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_millis(100))
                .await;

            assert_eq!(result.unwrap_err().code, "engine_timeout");
            assert!(started.elapsed() < Duration::from_secs(5));
            child.shutdown().await;
        });
}

#[test]
fn response_pair_and_revision_must_match_the_request() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("wrong-pair");
            let payload = serde_json::json!({
                "sync_pair_id": "00000000-0000-0000-0000-000000000001",
                "processing_revision": "00000000-0000-0000-0000-000000000003",
                "text": "harmless"
            });

            let result: Result<serde_json::Value, _> = child
                .request("preview_rules", &payload, Duration::from_secs(1))
                .await;

            assert_eq!(result.unwrap_err().code, "invalid_sidecar_protocol");
            child.shutdown().await;
        });
}

#[test]
fn unknown_response_id_is_ignored_without_stealing_the_current_reply() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("wrong-id");

            let response: serde_json::Value = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(1))
                .await
                .unwrap();

            assert_eq!(response, serde_json::json!({}));
            child.shutdown().await;
        });
}

#[test]
fn oversized_frame_is_rejected_before_deserialization() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("oversized");

            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(5))
                .await;

            assert_eq!(result.unwrap_err().code, "message_too_large");
            child.shutdown().await;
        });
}

#[test]
fn response_envelope_rejects_unknown_fields() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("extra-envelope");

            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(1))
                .await;

            assert_eq!(result.unwrap_err().code, "invalid_sidecar_protocol");
            child.shutdown().await;
        });
}

#[test]
fn sidecar_error_exposes_only_its_safe_payload() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("safe-error");

            let error = child
                .request::<_, serde_json::Value>(
                    "ping",
                    &serde_json::json!({}),
                    Duration::from_secs(1),
                )
                .await
                .unwrap_err();

            assert_eq!(error.code, "invalid_configuration");
            assert!(!error.retryable);
            child.shutdown().await;
        });
}

#[test]
fn sidecar_error_code_must_match_the_safe_code_contract() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("unsafe-error");
            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(1))
                .await;
            assert_eq!(result.unwrap_err().code, "invalid_sidecar_protocol");
            child.shutdown().await;
        });
}

#[test]
fn unexpected_exit_is_reaped_and_retried_once_with_a_fresh_id() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("exit-marker");
            let child = fake_with_marker("exit-once", &marker);

            let response: serde_json::Value = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(2))
                .await
                .unwrap();

            let first_id = std::fs::read_to_string(marker).unwrap();
            assert!(Uuid::parse_str(&first_id).is_ok());
            assert!(Uuid::parse_str(response["retry_id"].as_str().unwrap()).is_ok());
            assert_ne!(response["retry_id"], first_id);
            child.shutdown().await;
        });
}

#[test]
fn document_retry_replays_the_complete_successful_configuration() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("restart-marker");
            let child = fake_with_marker("exit-once-configured", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";
            let configuration = serde_json::json!({
                "sync_pair_id": pair,
                "processing_revision": revision,
                "config": {
                    "model": "de_core_news_sm",
                    "enabled_entities": ["PERSON", "EMAIL_ADDRESS"],
                    "custom_rules": [{
                        "id": "00000000-0000-0000-0000-000000000005",
                        "entity_type": "CUSTOM",
                        "enabled": true,
                        "kind": "words",
                        "words": ["SYNTHETIC_TERM"]
                    }],
                    "include_positions": false
                }
            });
            child
                .request::<_, serde_json::Value>(
                    "configure",
                    &configuration,
                    Duration::from_secs(2),
                )
                .await
                .unwrap();

            let response: serde_json::Value = child
                .request(
                    "process_document",
                    &serde_json::json!({
                        "sync_pair_id": pair,
                        "processing_revision": revision,
                        "source_path": "synthetic.docx"
                    }),
                    Duration::from_secs(2),
                )
                .await
                .unwrap();

            assert_eq!(response["configured_model"], "de_core_news_sm");
            let replayed: Vec<_> = std::fs::read_to_string(marker.with_extension("configs"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .collect();
            assert_eq!(replayed, vec![configuration.clone(), configuration]);
            child.shutdown().await;
        });
}

#[test]
fn fresh_process_restores_configuration_after_timeout_and_ping() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("timed-process");
            let child = fake_with_marker("timeout-then-require-config", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";
            let configuration = configuration(pair, revision);
            child
                .request::<_, ConfigureResult>("configure", &configuration, Duration::from_secs(1))
                .await
                .unwrap();

            let timed_out: Result<serde_json::Value, _> = child
                .request(
                    "process_document",
                    &document(pair, revision),
                    Duration::from_millis(100),
                )
                .await;
            assert_eq!(timed_out.unwrap_err().code, "engine_timeout");
            child
                .request::<_, serde_json::Value>(
                    "ping",
                    &serde_json::json!({}),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();

            let processed: serde_json::Value = child
                .request(
                    "process_document",
                    &document(pair, revision),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
            assert_eq!(processed["configured_model"], "de_core_news_sm");
            assert_eq!(
                std::fs::read_to_string(marker.with_extension("configs"))
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
            child.shutdown().await;
        });
}

#[test]
fn shutdown_during_configuration_replay_is_classified_as_cancellation() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("cancel-replay");
            let child = fake_with_marker("replay-hang", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";
            child
                .request::<_, ConfigureResult>(
                    "configure",
                    &configuration(pair, revision),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
            child.shutdown().await;

            let request_child = child.clone();
            let request = tokio::spawn(async move {
                request_child
                    .request::<_, serde_json::Value>(
                        "process_document",
                        &document(pair, revision),
                        Duration::from_secs(10),
                    )
                    .await
            });
            wait_for_marker(&marker).await;
            child.shutdown().await;

            let error = request.await.unwrap().unwrap_err();
            assert_eq!(error.code, "operation_cancelled");
        });
}

#[test]
fn first_exit_during_initial_replay_uses_the_single_restart_budget() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("one-replay-exit");
            let child = fake_with_marker("replay-exit-once", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";
            child
                .request::<_, ConfigureResult>(
                    "configure",
                    &configuration(pair, revision),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
            child.shutdown().await;

            let processed: serde_json::Value = child
                .request(
                    "process_document",
                    &document(pair, revision),
                    Duration::from_secs(2),
                )
                .await
                .unwrap();
            assert_eq!(processed["configured_model"], "de_core_news_sm");
            assert_eq!(
                std::fs::read_to_string(marker.with_extension("configs"))
                    .unwrap()
                    .lines()
                    .count(),
                3
            );
            child.shutdown().await;
        });
}

#[test]
fn second_exit_during_replay_stops_without_a_third_attempt() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("two-replay-exits");
            let child = fake_with_marker("replay-exit-twice", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";
            child
                .request::<_, ConfigureResult>(
                    "configure",
                    &configuration(pair, revision),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
            child.shutdown().await;

            let processed: Result<serde_json::Value, _> = child
                .request(
                    "process_document",
                    &document(pair, revision),
                    Duration::from_secs(2),
                )
                .await;
            assert_eq!(processed.unwrap_err().code, "engine_unavailable");
            assert_eq!(
                std::fs::read_to_string(marker.with_extension("configs"))
                    .unwrap()
                    .lines()
                    .count(),
                3
            );
            child.shutdown().await;
        });
}

#[test]
fn invalid_configure_result_is_not_saved_or_replayed() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("invalid-initial-config");
            let child = fake_with_marker("invalid-configure-then-exit", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";

            let configured: Result<serde_json::Value, _> = child
                .request(
                    "configure",
                    &configuration(pair, revision),
                    Duration::from_secs(1),
                )
                .await;
            assert_eq!(configured.unwrap_err().code, "invalid_sidecar_protocol");
            let processed: Result<serde_json::Value, _> = child
                .request(
                    "process_document",
                    &document(pair, revision),
                    Duration::from_secs(1),
                )
                .await;
            assert_eq!(processed.unwrap_err().code, "configuration_mismatch");
            assert_eq!(
                std::fs::read_to_string(marker.with_extension("configs"))
                    .unwrap()
                    .lines()
                    .count(),
                1
            );
            child.shutdown().await;
        });
}

#[test]
fn invalid_configure_result_during_replay_stops_before_document_retry() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("invalid-replay-config");
            let child = fake_with_marker("invalid-configure-replay", &marker);
            let pair = "00000000-0000-0000-0000-000000000001";
            let revision = "00000000-0000-0000-0000-000000000003";
            child
                .request::<_, ConfigureResult>(
                    "configure",
                    &configuration(pair, revision),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();

            let processed: Result<serde_json::Value, _> = child
                .request(
                    "process_document",
                    &document(pair, revision),
                    Duration::from_secs(1),
                )
                .await;
            assert_eq!(processed.unwrap_err().code, "invalid_sidecar_protocol");
            assert_eq!(
                std::fs::read_to_string(marker.with_extension("configs"))
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
            child.shutdown().await;
        });
}

#[test]
fn shutdown_cancels_a_request_during_startup_without_restarting() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("hang");
            let request_child = child.clone();
            let request = tokio::spawn(async move {
                request_child
                    .request::<_, serde_json::Value>(
                        "ping",
                        &serde_json::json!({}),
                        Duration::from_secs(10),
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(50)).await;

            child.shutdown().await;

            let error = tokio::time::timeout(Duration::from_secs(2), request)
                .await
                .expect("cancelled request must finish promptly")
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, "operation_cancelled");
        });
}

#[test]
fn shutdown_allows_graceful_eof_before_forcing_exit() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("eof-marker");
            let child = fake_with_marker("eof-marker", &marker);
            child
                .request::<_, serde_json::Value>(
                    "ping",
                    &serde_json::json!({}),
                    Duration::from_secs(1),
                )
                .await
                .unwrap();

            child.shutdown().await;

            assert_eq!(std::fs::read_to_string(marker).unwrap(), "eof");
        });
}

#[test]
fn rust_configuration_payload_matches_the_python_contract() {
    let payload = ConfigurePayload {
        sync_pair_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        processing_revision: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
        config: ProcessingConfig::default(),
    };

    let encoded = serde_json::to_value(payload).unwrap();

    assert_eq!(
        encoded["sync_pair_id"],
        "00000000-0000-0000-0000-000000000001"
    );
    assert_eq!(
        encoded["processing_revision"],
        "00000000-0000-0000-0000-000000000003"
    );
    assert_eq!(
        encoded["config"]["enabled_entities"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(encoded["config"]["include_positions"], true);
}

#[test]
fn process_result_enforces_python_identity_hash_timestamp_and_scalar_constraints() {
    let valid = serde_json::json!({
        "sync_pair_id": "00000000-0000-0000-0000-000000000001",
        "doc_id": "doc-0001",
        "source_hash_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "processing_revision": "00000000-0000-0000-0000-000000000003",
        "redacted_at": "2026-09-19t12:34:56z",
        "markdown": "",
        "body": "",
        "original_text": "Anna",
        "detections": [{
            "id": "opaque-1",
            "start": 0,
            "end": 4,
            "entity_type": "PERSON",
            "confidence": 1.0,
            "recognizer": "SpacyRecognizer",
            "origin": "automatic"
        }],
        "redactions": [],
        "warnings": [],
        "body_was_empty": false,
        "review_status": "pending",
        "engine": {
            "engine_version": "redactio-sidecar 0.1.0",
            "model_name": "de_core_news_lg",
            "model_version": "3.8.0",
            "recognizers": ["SpacyRecognizer"],
            "extraction_version": "1"
        }
    });
    assert!(serde_json::from_value::<ProcessResult>(valid.clone()).is_ok());

    for invalid in [
        (
            "sync_pair_id",
            serde_json::json!("00000000000000000000000000000001"),
        ),
        ("source_hash_sha256", serde_json::json!("A".repeat(64))),
        ("redacted_at", serde_json::json!("2026-09-19T12:34:56")),
        ("body_was_empty", serde_json::json!(0)),
    ] {
        let mut candidate = valid.clone();
        candidate[invalid.0] = invalid.1;
        assert!(serde_json::from_value::<ProcessResult>(candidate).is_err());
    }

    let mut invalid_offset = valid;
    invalid_offset["detections"][0]["start"] = serde_json::json!("0");
    assert!(serde_json::from_value::<ProcessResult>(invalid_offset).is_err());
}

#[test]
fn nullable_protocol_fields_are_required_even_when_null_is_allowed() {
    let detection = serde_json::json!({
        "id": "opaque-1",
        "start": 0,
        "end": 4,
        "entity_type": "PERSON",
        "confidence": null,
        "recognizer": "synthetic-recognizer",
        "origin": "automatic"
    });
    assert!(serde_json::from_value::<Detection>(detection.clone()).is_ok());
    let mut missing_detection_confidence = detection;
    missing_detection_confidence
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    assert!(serde_json::from_value::<Detection>(missing_detection_confidence).is_err());

    let output = serde_json::json!({
        "start_offset": 0,
        "end_offset": 4,
        "entity_type": "PERSON",
        "placeholder": "<PERSON_1>",
        "confidence": null,
        "recognizer": "synthetic-recognizer",
        "origin": "automatic"
    });
    assert!(serde_json::from_value::<OutputEntry>(output.clone()).is_ok());
    let mut missing_output_confidence = output;
    missing_output_confidence
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    assert!(serde_json::from_value::<OutputEntry>(missing_output_confidence).is_err());

    let review = serde_json::json!({
        "sync_pair_id": "00000000-0000-0000-0000-000000000001",
        "doc_id": "doc-0001",
        "source_hash_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "processing_revision": "00000000-0000-0000-0000-000000000003",
        "redacted_at": "2026-09-19T12:34:56Z",
        "source_path": "synthetic.docx",
        "detections": [],
        "decisions": {"dismissed_ids": [], "manual": []},
        "review_status": "pending",
        "reviewed_at": null,
        "acknowledged_warnings": []
    });
    assert!(serde_json::from_value::<ReviewRequest>(review.clone()).is_ok());
    let mut missing_reviewed_at = review;
    missing_reviewed_at
        .as_object_mut()
        .unwrap()
        .remove("reviewed_at");
    assert!(serde_json::from_value::<ReviewRequest>(missing_reviewed_at).is_err());
}

#[test]
#[cfg(debug_assertions)]
fn resource_resolution_uses_explicit_development_process_arguments() {
    static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _environment = ENVIRONMENT.lock().unwrap();
    let executable = std::env::current_exe().unwrap();
    let model_root = tempfile::tempdir().unwrap();
    std::env::set_var("REDACTIO_SIDECAR_EXECUTABLE", &executable);
    std::env::set_var("REDACTIO_SIDECAR_ARGS_JSON", r#"["-m","redactio_sidecar"]"#);
    std::env::set_var("REDACTIO_MODEL_DIR", model_root.path());

    let resolved = resources::resolve();
    std::env::set_var("REDACTIO_SIDECAR_ARGS_JSON", "not-json");
    let malformed = resources::resolve();
    std::env::remove_var("REDACTIO_MODEL_DIR");
    let incomplete = resources::resolve();
    std::env::set_var("REDACTIO_MODEL_DIR", "relative-models");
    let relative = resources::resolve();

    std::env::remove_var("REDACTIO_SIDECAR_EXECUTABLE");
    std::env::remove_var("REDACTIO_SIDECAR_ARGS_JSON");
    std::env::remove_var("REDACTIO_MODEL_DIR");
    assert!(malformed.is_err());
    assert!(incomplete.is_err());
    assert!(relative.is_err());
    let resolved = resolved.unwrap();
    assert_eq!(resolved.sidecar_executable, executable);
    assert_eq!(
        resolved.sidecar_args,
        vec![OsString::from("-m"), OsString::from("redactio_sidecar")]
    );
    assert_eq!(resolved.model_root, model_root.path());
}

#[test]
fn request_type_is_limited_to_the_c2_protocol() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("echo");
            let payload = serde_json::json!({
                "sync_pair_id": "00000000-0000-0000-0000-000000000001",
                "processing_revision": "00000000-0000-0000-0000-000000000003"
            });

            let result: Result<serde_json::Value, _> = child
                .request("private_extension", &payload, Duration::from_secs(1))
                .await;

            assert_eq!(result.unwrap_err().code, "invalid_sidecar_request");
            child.shutdown().await;
        });
}

#[test]
fn model_root_is_passed_as_an_explicit_child_argument() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("check-model-arg");

            let response: serde_json::Value = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(1))
                .await
                .unwrap();

            assert_eq!(response, serde_json::json!({}));
            child.shutdown().await;
        });
}

#[test]
fn second_child_exit_fails_without_a_third_spawn() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("exit-count");
            let child = fake_with_marker("exit-always", &marker);

            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(2))
                .await;

            assert_eq!(result.unwrap_err().code, "engine_unavailable");
            assert_eq!(std::fs::read_to_string(marker).unwrap(), "2");
            child.shutdown().await;
        });
}

#[test]
fn timeout_kills_without_automatic_retry() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("start-count");
            let child = fake_with_marker("hang-count", &marker);

            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_millis(100))
                .await;

            assert_eq!(result.unwrap_err().code, "engine_timeout");
            assert_eq!(std::fs::read_to_string(marker).unwrap(), "1");
            child.shutdown().await;
        });
}

#[test]
fn restart_keeps_the_original_whole_request_deadline() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("slow-exit");
            let child = fake_with_marker("slow-exit-once", &marker);
            let started = Instant::now();

            let result: Result<serde_json::Value, _> = child
                .request("ping", &serde_json::json!({}), Duration::from_millis(130))
                .await;

            assert_eq!(result.unwrap_err().code, "engine_timeout");
            assert!(started.elapsed() < Duration::from_secs(1));
            child.shutdown().await;
        });
}

#[test]
fn arbitrary_library_stderr_is_discarded_without_blocking_or_exposure() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("stderr-canary");

            let response: serde_json::Value = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(2))
                .await
                .unwrap();

            assert_eq!(response, serde_json::json!({}));
            child.shutdown().await;
        });
}

#[test]
fn wrong_response_type_and_excessive_json_depth_fail_safely() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for mode in ["wrong-type", "deeply-nested"] {
                let child = fake(mode);
                let result: Result<serde_json::Value, _> = child
                    .request("ping", &serde_json::json!({}), Duration::from_secs(1))
                    .await;
                assert_eq!(result.unwrap_err().code, "invalid_sidecar_protocol");
                child.shutdown().await;
            }
        });
}

#[test]
fn outbound_message_limit_is_enforced_before_write() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("echo");
            let payload = serde_json::json!({"padding": "x".repeat(64 * 1024 * 1024)});

            let result: Result<serde_json::Value, _> = child
                .request("ping", &payload, Duration::from_secs(2))
                .await;

            assert_eq!(result.unwrap_err().code, "message_too_large");
            child.shutdown().await;
        });
}

#[test]
fn child_does_not_inherit_python_module_override_paths() {
    static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _environment = ENVIRONMENT.lock().unwrap();
    std::env::set_var("PYTHONPATH", "/tmp/CANARY_PRIVATE_PATH");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = fake("clean-python-env");
            let response: serde_json::Value = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(1))
                .await
                .unwrap();
            assert_eq!(response, serde_json::json!({}));
            child.shutdown().await;
        });
    std::env::remove_var("PYTHONPATH");
}

#[test]
fn python_contract_emitter_roundtrips_through_rust_and_the_python_validator() {
    let python = std::env::var_os("REDACTIO_TEST_PYTHON")
        .expect("Set REDACTIO_TEST_PYTHON to the test interpreter's absolute path");
    let emitter = common::manifest_dir().join("../../sidecar/tests/test_contract.py");
    let output = Command::new(&python)
        .arg(&emitter)
        .arg("--emit")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let messages: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 11);

    for message in &messages[5..] {
        let payload = message["payload"].clone();
        match message["type"].as_str().unwrap() {
            "ping_result" => {
                assert_eq!(
                    serde_json::from_value::<PingResult>(payload)
                        .unwrap()
                        .protocol_version,
                    1
                )
            }
            "configure_result" => {
                serde_json::from_value::<ConfigureResult>(payload).unwrap();
            }
            "process_document_result" | "render_review_result" => {
                serde_json::from_value::<ProcessResult>(payload).unwrap();
            }
            "preview_rules_result" => {
                serde_json::from_value::<PreviewRulesResult>(payload).unwrap();
            }
            "error" => {
                serde_json::from_value::<redactio_lib::error::AppError>(payload).unwrap();
            }
            kind => panic!("unexpected emitted response: {kind}"),
        }
    }

    let mut validator = Command::new(python)
        .args([
            "-c",
            "import sys; from pydantic import TypeAdapter; from redactio_sidecar.schemas import Request; adapter=TypeAdapter(Request); [adapter.validate_json(line) for line in sys.stdin.buffer]",
        ])
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = validator.stdin.as_mut().unwrap();
    for message in &messages[..5] {
        let payload = match message["type"].as_str().unwrap() {
            "ping" => serde_json::json!({}),
            "configure" => serde_json::to_value(
                serde_json::from_value::<ConfigurePayload>(message["payload"].clone()).unwrap(),
            )
            .unwrap(),
            "process_document" => serde_json::to_value(
                serde_json::from_value::<ProcessRequest>(message["payload"].clone()).unwrap(),
            )
            .unwrap(),
            "preview_rules" => serde_json::to_value(
                serde_json::from_value::<PreviewRulesPayload>(message["payload"].clone()).unwrap(),
            )
            .unwrap(),
            "render_review" => serde_json::to_value(
                serde_json::from_value::<ReviewRequest>(message["payload"].clone()).unwrap(),
            )
            .unwrap(),
            kind => panic!("unexpected emitted request: {kind}"),
        };
        serde_json::to_writer(
            &mut *stdin,
            &serde_json::json!({
                "id": message["id"],
                "type": message["type"],
                "payload": payload
            }),
        )
        .unwrap();
        stdin.write_all(b"\n").unwrap();
    }
    drop(validator.stdin.take());
    let validated = validator.wait_with_output().unwrap();
    assert!(
        validated.status.success(),
        "Python rejected Rust messages: {}",
        String::from_utf8_lossy(&validated.stderr)
    );
}

#[test]
#[ignore = "requires the reviewed sidecar interpreter and bundled models"]
fn reviewed_cli_roundtrips_through_the_bundled_model() {
    let python = std::env::var_os("REDACTIO_REAL_SIDECAR_PYTHON")
        .expect("Set REDACTIO_REAL_SIDECAR_PYTHON to the reviewed sidecar interpreter");
    let model_root = std::env::var_os("REDACTIO_REAL_MODEL_DIR")
        .expect("Set REDACTIO_REAL_MODEL_DIR to the bundled model root");
    let pair = Uuid::new_v4();
    let revision = Uuid::new_v4();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = Sidecar::new(
                PathBuf::from(python),
                vec![OsString::from("-m"), OsString::from("redactio_sidecar")],
                PathBuf::from(model_root),
            );

            let ping: PingResult = child
                .request("ping", &serde_json::json!({}), Duration::from_secs(10))
                .await
                .unwrap();
            assert_eq!(ping.protocol_version, 1);

            let configured: ConfigureResult = child
                .request(
                    "configure",
                    &ConfigurePayload {
                        sync_pair_id: pair,
                        processing_revision: revision,
                        config: ProcessingConfig::default(),
                    },
                    redactio_lib::sidecar::INITIALIZATION_TIMEOUT,
                )
                .await
                .unwrap();
            assert_eq!(configured.sync_pair_id, pair);
            assert_eq!(configured.processing_revision, revision);
            assert_eq!(
                configured.engine.model_name,
                "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
            );

            let preview: PreviewRulesResult = child
                .request(
                    "preview_rules",
                    &PreviewRulesPayload {
                        sync_pair_id: pair,
                        processing_revision: revision,
                        text: "Max Mustermann wohnt in Berlin.".into(),
                    },
                    redactio_lib::sidecar::DOCUMENT_TIMEOUT,
                )
                .await
                .unwrap();
            assert_eq!(preview.sync_pair_id, pair);
            assert_eq!(preview.processing_revision, revision);
            assert!(preview
                .detections
                .iter()
                .any(|detection| detection.entity_type
                    == redactio_lib::domain::settings::EntityType::Person));
            assert!(preview
                .detections
                .iter()
                .any(|detection| detection.entity_type
                    == redactio_lib::domain::settings::EntityType::Location));
            child.shutdown().await;
        });
}
