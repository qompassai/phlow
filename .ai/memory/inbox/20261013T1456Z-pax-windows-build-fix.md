# Windows (windows-gnu) build of phlow-cli unblocked

Found by: Batch 3 release-binaries build at 670dd5d (14 E0432/E0433 in
phlow-workspace; further blockers surfaced one at a time as each was fixed).

Root causes, all one shape — Unix-only machinery compiled unconditionally:

- phlow-workspace: rustix::fs is #[cfg(not(windows))] in rustix 1.1.5.
  Fix: descriptor-relative machinery (open_parent/write_temp/syscall_err/
  identity_matches/temp names/mode consts) is cfg(unix); read/write split
  into read_file/write_file with non-Unix variants returning the
  pre-existing WorkspaceError::Unavailable contract. No std-based
  substitute: std cannot express O_NOFOLLOW/dirfd-relative opens (see the
  Cargo.toml comment), so a Windows read/write would silently weaken
  containment. rustix is now a [target.cfg(unix).dependencies] dep.
- phlow-runtime transport/msgpack.rs: tokio::net::UnixStream is
  cfg(all(unix, feature = "net")). Socket machinery (worker_loop/
  service_request/WorkerStream/read_frame/read_response/decode_response)
  is cfg(unix); the non-Unix worker_loop answers every request with
  TransportError::Failed("Neovim editor socket requires POSIX").
  encode_request stays cfg(any(unix, test)) — the tests exercise it on
  every platform. new()/exec()/close() are platform-identical.
- phlow-cli: signal-hook moved to cfg(unix) target deps (the watcher in
  src/signal.rs was already cfg(unix)); EXIT_INTERRUPTED and
  INTERRUPTED_MESSAGE are cfg(any(unix, test)) — consumed only by the
  watcher and the byte-parity tests.

Validation (primo, pinned nightly-2026-09-25):
- cargo test with GAUNTLET_NVIM_BIN=/usr/bin/nvim: 2588 passed /
  0 failed / 29 ignored — identical to the pre-fix baseline.
- cargo build --release -p phlow-cli: x86_64 EXIT 0, aarch64 EXIT 0,
  x86_64-pc-windows-gnu EXIT 0, zero warnings on all three.
- phlow.exe: PE32+ executable for MS Windows (cannot run on primo).
- Headless nvim (diver config) diagnostics on the three changed .rs
  files: 0 errors, 0 total. rustfmt --check clean on the same files.

Build note: aarch64 linking needs
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
in the environment; without it the final link fails with
"linking with cc failed" after a clean compile.
