//! Egress filter for phlow check children (Linux seccomp-bpf).
//!
//! [`apply_egress_filter`] arranges for a [`Command`]'s child to run under a
//! seccomp filter, installed between fork and exec and inherited by every
//! descendant across `exec`:
//!
//! - `socket(AF_INET | AF_INET6, ..)` fails with `EPERM`, whatever the
//!   socket type or protocol. The family is compared as the kernel reads it
//!   (a 32-bit `int`), so garbage in the upper half of the register cannot
//!   slip past.
//! - `io_uring_setup` fails with `EPERM`, closing the `IORING_OP_SOCKET`
//!   route (Linux 5.19+) to the same sockets.
//! - A syscall made under a different architecture (a 64-bit child using
//!   the 32-bit `int 0x80` entry) kills the process: seccompiler's arch
//!   check. On x86_64 the x32 aliases (`nr | 0x4000_0000`) of both denied
//!   syscalls are denied too, since x32 shares `AUDIT_ARCH_X86_64`.
//! - Everything else is allowed, including `AF_UNIX` sockets.
//!
//! Installing a filter unprivileged requires `PR_SET_NO_NEW_PRIVS`, so a
//! check can never gain privilege through `execve`: setuid/setgid bits and
//! file capabilities are ignored (`sudo` inside a check fails). That is
//! intended; checks must never gain privilege.
//!
//! # Known limitations (phlow is not an OS sandbox)
//!
//! This denies creating IP sockets; it does not stop every route off the
//! host. Not covered: talking over `AF_UNIX` to a local process that does
//! have network access (a proxy, a container daemon socket), `AF_VSOCK`,
//! `AF_NETLINK`, an IP socket handed over with `SCM_RIGHTS` by an unfiltered
//! process, or any other process the child can ask to connect on its
//! behalf. Descriptors inherited from phlow are not a route: std opens
//! everything close-on-exec and the child gets only null stdin and two
//! pipes.
//!
//! # Platforms
//!
//! Linux only. Elsewhere [`apply_egress_filter`] does nothing and returns
//! `Ok` (no filter is available; documented, not failed closed). On Linux
//! architectures seccompiler cannot target (anything but x86_64, aarch64 and
//! riscv64), it returns an error, so the check does not run unfiltered;
//! big-endian Linux does not build (seccompiler is little-endian only).
//!
//! # Unsafe boundary
//!
//! The crate's only `unsafe` is the [`CommandExt::pre_exec`] registration
//! in `register`. The BPF program is compiled in the parent; after fork the
//! child only calls `prctl(PR_SET_NO_NEW_PRIVS)` and
//! `seccomp(SECCOMP_SET_MODE_FILTER)` on that already-built program — raw
//! syscalls, no allocation, no locks (see `install`). Real-child tests of
//! the public API live in `tests/egress.rs`.
//!
//! [`CommandExt::pre_exec`]: std::os::unix::process::CommandExt::pre_exec

#![deny(unsafe_code)]

use std::io;
use std::process::Command;

/// Arrange for `command`'s child to run under the egress filter.
///
/// The filter is compiled here, in the parent; spawning then installs it
/// in the child before `exec`. If installation fails, `Command::spawn`
/// returns the error and no child program runs, so a filtered command never
/// runs unfiltered.
///
/// # Errors
///
/// The host architecture has no seccompiler target, or the filter does not
/// compile. The command is left unmodified.
#[cfg(target_os = "linux")]
pub fn apply_egress_filter(command: &mut Command) -> io::Result<()> {
    let program = linux::egress_program()?;
    linux::register(command, program);
    Ok(())
}

/// Non-Linux: seccomp does not exist, so no filter is installed. See the
/// crate docs.
#[cfg(not(target_os = "linux"))]
pub fn apply_egress_filter(_command: &mut Command) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::io;
    use std::process::Command;

    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, TargetArch, sock_filter,
    };

    /// Syscall-number variants to deny. x86_64 kernels built with
    /// `CONFIG_X86_X32_ABI` also dispatch `nr | __X32_SYSCALL_BIT`
    /// (arch/x86/include/uapi/asm/unistd.h) under the same audit arch.
    #[cfg(target_arch = "x86_64")]
    const SYSCALL_ALIAS_BITS: [i64; 2] = [0, 0x4000_0000];
    #[cfg(not(target_arch = "x86_64"))]
    const SYSCALL_ALIAS_BITS: [i64; 1] = [0];

    /// Denied address families: `socket()`'s first argument.
    const DENIED_FAMILIES: [libc::c_int; 2] = [libc::AF_INET, libc::AF_INET6];

    /// Compile the egress filter for the host architecture.
    pub(crate) fn egress_program() -> io::Result<BpfProgram> {
        let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(io::Error::other)?;
        let mut family_rules = Vec::with_capacity(DENIED_FAMILIES.len());
        for family in DENIED_FAMILIES {
            family_rules.push(family_rule(family)?);
        }
        let mut rules = BTreeMap::new();
        for bit in SYSCALL_ALIAS_BITS {
            rules.insert(libc::SYS_socket | bit, family_rules.clone());
            // An empty rule list matches the syscall unconditionally.
            rules.insert(libc::SYS_io_uring_setup | bit, Vec::new());
        }
        let denied = SeccompAction::Errno(libc::EPERM as u32);
        let filter = SeccompFilter::new(rules, SeccompAction::Allow, denied, arch)
            .map_err(io::Error::other)?;
        let program = BpfProgram::try_from(filter).map_err(io::Error::other)?;
        // seccompiler emits an arch check before any rule, never an empty program.
        assert!(!program.is_empty(), "compiled egress filter is empty");
        Ok(program)
    }

    /// `socket(family, ..)`: argument 0 compared as the kernel's `int`.
    fn family_rule(family: libc::c_int) -> io::Result<SeccompRule> {
        let value = u64::try_from(family).map_err(io::Error::other)?;
        let condition = SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, value)
            .map_err(io::Error::other)?;
        SeccompRule::new(vec![condition]).map_err(io::Error::other)
    }

    /// Register `program` for installation in `command`'s child.
    #[allow(unsafe_code)]
    pub(crate) fn register(command: &mut Command, program: BpfProgram) {
        use std::os::unix::process::CommandExt;
        assert!(!program.is_empty(), "refusing to register an empty filter");
        // SAFETY: `pre_exec` requires the closure to be safe to run in the
        // forked child of a possibly multi-threaded parent: async-signal-safe
        // operations only, no allocation, no locks. The closure owns
        // `program`, allocated here in the parent and only read in the child.
        // `install` makes two raw syscalls (prctl, seccomp) through
        // seccompiler, which passes a stack `sock_fprog` pointing at that
        // slice, and builds errors only with `io::Error::last_os_error` /
        // `from_raw_os_error`, neither of which allocates.
        unsafe {
            command.pre_exec(move || install(&program));
        }
    }

    /// Runs in the child between fork and exec. Must not allocate.
    fn install(program: &[sock_filter]) -> io::Result<()> {
        match seccompiler::apply_filter(program) {
            Ok(()) => Ok(()),
            Err(seccompiler::Error::Prctl(error) | seccompiler::Error::Seccomp(error)) => {
                Err(error)
            }
            // apply_filter produces only Prctl/Seccomp here: the program is
            // nonempty (asserted at registration) and TSYNC is not requested.
            // Keep the failure an errno so std reports it from spawn.
            Err(_) => Err(io::Error::from_raw_os_error(libc::EINVAL)),
        }
    }
}
