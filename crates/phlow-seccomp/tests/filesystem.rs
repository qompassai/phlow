#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use phlow_seccomp::{RunDirectory, confined_command};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(parent: &Path) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("phlow-fs-test-{}-{id}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn python(directory: RunDirectory<'_>, body: &str, args: &[&str]) {
    let output = confined_command(Command::new("/usr/bin/python3"), directory)
        .unwrap()
        .args(["-c", body])
        .args(args)
        .output()
        .expect("confined spawn");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn outside_paths_and_inherited_descriptor_are_denied() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let sentinel = Fixture::new(&home);
    let work = Fixture::new(Path::new("/tmp"));
    let private = sentinel.0.join("private");
    fs::write(&private, "fixture").unwrap();
    symlink(&private, work.0.join("home-link")).unwrap();

    let file = File::options().write(true).open(&private).unwrap();
    // F_DUPFD deliberately creates an inheritable descriptor. Keep the
    // descriptor owned until spawn and verify it never changes the fixture.
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 100) };
    assert!(fd >= 100);
    let inheritable = unsafe { File::from_raw_fd(fd) };
    let proc_path = format!(
        "/proc/self/cwd/../{}",
        private.strip_prefix("/").unwrap().display()
    );
    let outside = sentinel.0.join("outside");
    python(
        RunDirectory::Writable(&work.0),
        r#"
import errno, os, sys
from pathlib import Path

def denied(action, errors):
    try:
        action()
    except OSError as error:
        assert error.errno in errors, error
    else:
        raise AssertionError("operation unexpectedly succeeded")

missing_or_denied = (errno.ENOENT, errno.EACCES)
denied(lambda: os.listdir(sys.argv[1]), missing_or_denied)
denied(lambda: Path(sys.argv[2]).read_bytes(), missing_or_denied)
denied(lambda: Path(sys.argv[3]).write_bytes(b'x'), missing_or_denied)
assert Path('home-link').is_symlink()
denied(lambda: Path('home-link').read_bytes(), missing_or_denied)
denied(lambda: Path(sys.argv[4]).read_bytes(), missing_or_denied)
denied(lambda: os.write(int(sys.argv[5]), b'x'), (errno.EBADF,))
# Mounts still expose the synthetic root. Landlock denies listing it.
denied(lambda: os.listdir('/'), (errno.EACCES,))
"#,
        &[
            home.join(".ssh").to_str().unwrap(),
            private.to_str().unwrap(),
            outside.to_str().unwrap(),
            &proc_path,
            &fd.to_string(),
        ],
    );
    drop(inheritable);
    assert_eq!(fs::read_to_string(private).unwrap(), "fixture");
    assert!(!outside.exists());
}

#[test]
fn read_only_revision_can_compute_and_write_scratch_with_egress_filtered() {
    let input = Fixture::new(Path::new("/tmp"));
    let path = input.0.join("input");
    fs::write(&path, "21").unwrap();
    python(
        RunDirectory::ReadOnly(&input.0),
        r#"
import errno, os, socket, tempfile
from pathlib import Path

assert os.getcwd() == '/input'
source = Path('input')
value = int(source.read_text())
with tempfile.TemporaryDirectory() as scratch:
    assert scratch.startswith('/work/')
    result = Path(scratch) / 'result'
    result.write_text(str(value * 2))
    assert result.read_text() == '42'
try:
    source.write_text('tampered')
except OSError as error:
    assert error.errno in (errno.EACCES, errno.EROFS), error
else:
    raise AssertionError("input was writable")
for family in (socket.AF_INET, socket.AF_INET6):
    try:
        socket.socket(family)
    except OSError as error:
        assert error.errno == errno.EPERM, error
    else:
        raise AssertionError("IP socket opened")
"#,
        &[],
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "21");
}

#[test]
fn missing_input_fails_before_child_can_start() {
    let parent = Fixture::new(Path::new("/tmp"));
    let result = confined_command(
        Command::new("true"),
        RunDirectory::ReadOnly(&parent.0.join("missing")),
    );
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn replaced_input_fails_at_spawn() {
    let parent = Fixture::new(Path::new("/tmp"));
    let path = parent.0.join("input");
    fs::create_dir(&path).unwrap();
    let mut command =
        confined_command(Command::new("/usr/bin/true"), RunDirectory::ReadOnly(&path)).unwrap();
    fs::rename(&path, parent.0.join("old-input")).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(
        command.spawn().unwrap_err().raw_os_error(),
        Some(libc::ESTALE)
    );
}
