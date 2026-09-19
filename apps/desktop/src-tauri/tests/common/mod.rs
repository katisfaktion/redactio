use std::path::PathBuf;

pub fn manifest_dir() -> PathBuf {
    // Cross-built tests run with a native checkout/fixture mirror, not the build host's path.
    std::env::var_os("REDACTIO_TEST_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}
