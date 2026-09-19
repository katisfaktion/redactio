use redactio_lib::domain::{
    audit::{append_audit, AuditAction, AuditEntry},
    sync::{RunCounts, RunOutcome},
};
use std::fs;
use uuid::Uuid;

fn entry() -> AuditEntry {
    AuditEntry {
        schema_version: 1,
        sync_pair_id: Uuid::new_v4(),
        run_id: Uuid::new_v4(),
        action: AuditAction::Sync,
        started_at: "2026-09-19T12:00:00Z".into(),
        finished_at: "2026-09-19T12:01:00Z".into(),
        outcome: RunOutcome::Completed,
        counts: RunCounts::default(),
        processing_revision: Uuid::new_v4(),
        engine: None,
        error_codes: vec![],
    }
}

#[test]
fn torn_trailing_record_is_recovered_without_erasing_complete_records() {
    let root = tempfile::tempdir().unwrap();
    append_audit(root.path(), &entry()).unwrap();
    let path = root.path().join("audit-log.jsonl");
    let first = fs::read(&path).unwrap();
    let mut torn = first.clone();
    torn.extend_from_slice(b"{\"private_incomplete");
    fs::write(&path, torn).unwrap();
    append_audit(root.path(), &entry()).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(&first));
    let lines: Vec<AuditEntry> = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1].action, AuditAction::Recovery);
}

#[test]
fn malformed_complete_record_and_invalid_counts_are_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("audit-log.jsonl");
    fs::write(&path, b"{invalid}\ntrailing").unwrap();
    assert!(append_audit(root.path(), &entry()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"{invalid}\ntrailing");
    let mut invalid = entry();
    invalid.counts.discovered = 1;
    assert!(append_audit(root.path(), &invalid).is_err());
}

#[test]
fn concurrent_processes_append_complete_records() {
    let root = tempfile::tempdir().unwrap();
    append_audit(root.path(), &entry()).unwrap();
    let children: Vec<_> = (0..6)
        .map(|_| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "audit_child", "--nocapture"])
                .env("REDACTIO_AUDIT_TEST_DIR", root.path())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let text = fs::read_to_string(root.path().join("audit-log.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 181);
    for line in text.lines() {
        serde_json::from_str::<AuditEntry>(line).unwrap();
    }
}

#[test]
fn audit_child() {
    if let Some(root) = std::env::var_os("REDACTIO_AUDIT_TEST_DIR") {
        for _ in 0..30 {
            append_audit(std::path::Path::new(&root), &entry()).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn redirected_audit_never_modifies_another_file() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), b"CANARY_PRIVATE").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("audit-log.jsonl")).unwrap();
    assert!(append_audit(root.path(), &entry()).is_err());
    assert_eq!(fs::read(outside.path()).unwrap(), b"CANARY_PRIVATE");
}
