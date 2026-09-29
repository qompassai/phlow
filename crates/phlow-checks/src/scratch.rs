//! Per-check scratch directories: rootless, XDG-compliant, mode 0700.
//!
//! Before each spawn the runner creates a fresh directory that only this
//! user can enter and points the child's `TMPDIR` at it; the directory is
//! removed once the check has been reaped. Location, first match wins:
//!
//! 1. `$XDG_RUNTIME_DIR/phlow/<check-id>` when `XDG_RUNTIME_DIR` is set to
//!    an absolute path (relative or empty values are invalid per the XDG
//!    Base Directory spec and ignored);
//! 2. `$TMPDIR/phlow-$UID/<check-id>` when `TMPDIR` is absolute;
//! 3. `/tmp/phlow-$UID/<check-id>`.
//!
//! `<check-id>` is `check-<pid>-<sequence>`, never the check name, so no
//! configured string reaches the path. Nothing else is written: no dotfiles,
//! no system directories. Long-lived state belongs under `XDG_STATE_HOME`
//! and caches under `XDG_CACHE_HOME`; this module keeps neither.
//!
//! # Threat model
//!
//! Holds: the base and the per-check directory are verified — not a
//! symlink, a directory, owned by the current uid, permission bits exactly
//! 0700 — or the check does not run. That defeats another local user
//! pre-creating `/tmp/phlow-$UID` (squatting or a symlink redirect).
//! Removal is std's `remove_dir_all`, which does not follow symlinks, so a
//! check cannot turn cleanup against files outside its scratch directory.
//!
//! Where `XDG_RUNTIME_DIR` is the systemd per-user tmpfs (mounted
//! `nosuid,nodev`, mode 0700), a setuid binary or device node dropped there
//! is inert; together with `PR_SET_NO_NEW_PRIVS` from the Linux egress
//! filter, a check cannot escalate through setuid and, running unprivileged,
//! cannot write system directories. The `/tmp` fallbacks carry whatever
//! mount options the host gives them; this module does not check them.
//!
//! Does not hold: the child runs as the invoking user, so it can read and
//! write everything that user can — `$HOME`, other scratch directories,
//! the workspace (its cwd). A scratch directory is a tidy private `TMPDIR`,
//! not a same-uid isolation boundary; phlow is not an OS sandbox.

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Permission bits required on the base and per-check directories.
const PRIVATE_DIR_MODE: u32 = 0o700;
/// Permission, setuid/setgid and sticky bits compared against
/// [`PRIVATE_DIR_MODE`].
const MODE_BITS_MASK: u32 = 0o7777;
/// Last-resort temporary root when neither environment variable is usable.
const TMP_ROOT: &str = "/tmp";

/// Per-process sequence making each `<check-id>` unique.
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

/// A fresh private scratch directory for one check run. The owner holds it
/// until the child is reaped; dropping it removes the directory tree.
pub(crate) struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// Create a scratch directory under this user's base (see module docs).
    pub(crate) fn create() -> io::Result<Scratch> {
        let uid = rustix::process::getuid().as_raw();
        let base = scratch_base(
            std::env::var_os("XDG_RUNTIME_DIR"),
            std::env::var_os("TMPDIR"),
            uid,
        );
        Scratch::create_in(&base, uid)
    }

    /// Create a scratch directory in `base`, which must be (or become) a
    /// private directory owned by `uid`.
    fn create_in(base: &Path, uid: u32) -> io::Result<Scratch> {
        match std::fs::DirBuilder::new()
            .mode(PRIVATE_DIR_MODE)
            .create(base)
        {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        verify_private_dir(base, uid)?;
        let sequence = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("check-{}-{sequence}", std::process::id()));
        // Non-recursive create: fails if the path exists, so the directory
        // is always fresh.
        std::fs::DirBuilder::new()
            .mode(PRIVATE_DIR_MODE)
            .create(&path)?;
        // Own it before verifying, so a failed verification still removes it.
        let scratch = Scratch { path };
        verify_private_dir(&scratch.path, uid)?;
        Ok(scratch)
    }

    /// The scratch directory; valid until `self` is dropped.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best-effort: the check already ran and its report is final; a
        // leftover directory stays private (0700) and is not reused.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Resolve the base directory from the relevant environment values.
pub(crate) fn scratch_base(
    xdg_runtime_dir: Option<OsString>,
    tmpdir: Option<OsString>,
    uid: u32,
) -> PathBuf {
    if let Some(runtime) = absolute(xdg_runtime_dir) {
        return runtime.join("phlow");
    }
    let root = absolute(tmpdir).unwrap_or_else(|| PathBuf::from(TMP_ROOT));
    root.join(format!("phlow-{uid}"))
}

/// An environment path, only when set, nonempty and absolute.
fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Reject anything but a real directory owned by `uid` with mode 0700.
fn verify_private_dir(path: &Path, uid: u32) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    let mode = metadata.mode() & MODE_BITS_MASK;
    if metadata.file_type().is_dir() && metadata.uid() == uid && mode == PRIVATE_DIR_MODE {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{} is not a private directory (need a non-symlink directory, owner uid {uid}, \
             mode 0700; found uid {}, mode {mode:o})",
            path.display(),
            metadata.uid(),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh parent directory for one test's base.
    fn parent_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-scratch-test-{}-{}",
            std::process::id(),
            TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create parent dir");
        dir
    }

    fn uid() -> u32 {
        rustix::process::getuid().as_raw()
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).expect("stat").mode() & MODE_BITS_MASK
    }

    // --- validation ---------------------------------------------------------

    #[test]
    fn base_prefers_xdg_runtime_dir() {
        let base = scratch_base(Some("/run/user/1000".into()), Some("/var/tmp".into()), 1000);
        assert_eq!(base, Path::new("/run/user/1000/phlow"));
    }

    #[test]
    fn base_falls_back_to_tmpdir_then_tmp_when_xdg_is_unset() {
        let base = scratch_base(None, Some("/var/tmp".into()), 1000);
        assert_eq!(base, Path::new("/var/tmp/phlow-1000"));
        assert_eq!(scratch_base(None, None, 1000), Path::new("/tmp/phlow-1000"));
    }

    #[test]
    fn scratch_dir_is_created_0700_and_owned() {
        let parent = parent_dir();
        let base = parent.join("phlow");
        let scratch = Scratch::create_in(&base, uid()).expect("create scratch");
        assert_eq!(mode(&base), 0o700);
        assert_eq!(mode(scratch.path()), 0o700);
        assert_eq!(scratch.path().parent(), Some(base.as_path()));
        let owner = std::fs::metadata(scratch.path()).expect("stat").uid();
        assert_eq!(owner, uid());
        cleanup(&parent);
    }

    #[test]
    fn scratch_dirs_are_distinct_and_removed_on_drop() {
        let parent = parent_dir();
        let base = parent.join("phlow");
        let first = Scratch::create_in(&base, uid()).expect("first");
        let second = Scratch::create_in(&base, uid()).expect("second");
        assert_ne!(first.path(), second.path());
        let path = first.path().to_path_buf();
        std::fs::write(path.join("leftover"), "x").expect("write inside scratch");
        drop(first);
        assert!(!path.exists());
        assert!(second.path().is_dir());
        drop(second);
        cleanup(&parent);
    }

    // --- adversarial --------------------------------------------------------

    #[test]
    fn relative_or_empty_environment_paths_are_ignored() {
        let base = scratch_base(Some("run/user".into()), Some("".into()), 7);
        assert_eq!(base, Path::new("/tmp/phlow-7"));
        let base = scratch_base(Some("".into()), Some("tmp".into()), 7);
        assert_eq!(base, Path::new("/tmp/phlow-7"));
    }

    #[test]
    fn symlinked_base_is_rejected() {
        let parent = parent_dir();
        let target = parent.join("elsewhere");
        std::fs::create_dir(&target).expect("create target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        let base = parent.join("phlow");
        std::os::unix::fs::symlink(&target, &base).expect("plant symlink");
        let error = Scratch::create_in(&base, uid())
            .err()
            .expect("symlink must be refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_dir(&target).expect("list target").count(), 0);
        cleanup(&parent);
    }

    #[test]
    fn shared_mode_base_is_rejected() {
        let parent = parent_dir();
        let base = parent.join("phlow");
        std::fs::create_dir(&base).expect("create base");
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let error = Scratch::create_in(&base, uid())
            .err()
            .expect("0755 must be refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        cleanup(&parent);
    }

    #[test]
    fn foreign_owned_base_is_rejected() {
        // Stands in for another user squatting /tmp/phlow-$UID: the base is
        // not owned by the uid the scratch is created for.
        let parent = parent_dir();
        let base = parent.join("phlow");
        let error = Scratch::create_in(&base, uid().wrapping_add(1))
            .err()
            .expect("foreign owner must be refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        cleanup(&parent);
    }

    #[test]
    fn file_base_is_rejected() {
        let parent = parent_dir();
        let base = parent.join("phlow");
        std::fs::write(&base, "not a dir").expect("plant file");
        assert!(Scratch::create_in(&base, uid()).is_err());
        cleanup(&parent);
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
