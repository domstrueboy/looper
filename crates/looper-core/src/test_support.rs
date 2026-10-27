//! Helpers more than one module's tests need. Compiled for tests only.

use std::path::PathBuf;

/// A directory of the test's own under the system temp dir, emptied
/// first. Named per process, so two test runs at once don't share one.
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("looper-pedal-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
