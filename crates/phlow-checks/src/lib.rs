//! Named, operator-approved checks for the phlow agent runtime.
//!
//! A [`CheckRunner`](runner::CheckRunner) executes the exact argv from the
//! operator's TOML — no shell, no model-chosen command, cwd, or
//! environment. Each check runs to completion or its configured timeout;
//! on timeout the whole process group is killed and reaped, so no orphan
//! keeps running after the report. Each program is pinned by canonical
//! path and SHA-256 at admission ([`pin`]) and re-verified before spawn.
//! On Unix each run gets a private 0700 scratch directory as its `TMPDIR`
//! (`scratch`); on Linux the child also runs under `phlow-seccomp`'s egress
//! filter (no AF_INET/AF_INET6 sockets, no io_uring, no privilege gain).
//!
//! The legacy `shell_exec` and `lsp_check` tools are fail-closed here:
//! [`disabled`] preserves their exact denial behavior as typed values,
//! never as silently skipped execution.

#![forbid(unsafe_code)]

pub mod disabled;
pub mod pin;
pub mod runner;
#[cfg(unix)]
mod scratch;

pub use disabled::{DisabledError, DisabledTool, LSP_CHECK_DENIAL, SHELL_DENIAL};
pub use phlow_config::CheckKind;
pub use pin::{BinaryPin, CheckPins, PIN_BYTES_MAX, pin_checks};
pub use runner::{
    CheckReport, CheckRunner, CheckStatus, OUTPUT_BYTES_MAX, PinRefusal, RunAllReport,
};
