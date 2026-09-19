use redactio_lib::domain::sync::RunCounts;

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
