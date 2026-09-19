#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunCounts {
    pub discovered: u64,
    pub processed: u64,
    pub skipped: u64,
    pub failed: u64,
    pub unprocessed: u64,
    pub warned: u64,
}

impl RunCounts {
    /// Invalid overflowing states saturate; validate with `is_consistent` before publication.
    pub fn completed(&self) -> u64 {
        self.processed
            .saturating_add(self.skipped)
            .saturating_add(self.failed)
    }

    pub fn is_consistent(&self) -> bool {
        self.warned <= self.processed
            && self
                .processed
                .checked_add(self.skipped)
                .and_then(|completed| completed.checked_add(self.failed))
                .and_then(|completed| completed.checked_add(self.unprocessed))
                == Some(self.discovered)
    }
}
