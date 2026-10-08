//! Test-only temporary directory support. The workspace lockfile has
//! no `tempfile` crate and the scaffold adds no dependencies for
//! tests, so this is the minimal RAII equivalent: a unique directory
//! under the OS temp dir, removed on drop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory that deletes itself on drop.
pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// The directory's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Create a unique empty temporary directory tagged for the test.
pub(crate) fn test_dir(tag: &str) -> TestDir {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "phlow-autoresearch-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create test dir");
    TestDir { path }
}
