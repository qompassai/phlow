//! Real children under the real egress filter, through the public API only.
//!
//! Each probe prints `ok` when the operation succeeded or `errno=N` when it
//! failed; controls run the same probe unfiltered to prove each adversarial
//! test discriminates on this host.

#![cfg(target_os = "linux")]

use phlow_seccomp::apply_egress_filter;
use std::process::Command;

const PYTHON: &str = "/usr/bin/python3";
const EPERM: &str = "errno=1";
/// x86_64 `__X32_SYSCALL_BIT`.
#[cfg(target_arch = "x86_64")]
const X32_SYSCALL_BIT: i64 = 0x4000_0000;

fn probe(script: &str, filtered: bool) -> String {
    let mut command = Command::new(PYTHON);
    command.args(["-c", script]);
    if filtered {
        apply_egress_filter(&mut command).expect("filter compiles on this host");
    }
    let output = command.output().expect("spawn python3");
    assert!(output.status.success(), "probe crashed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("probe prints ASCII")
        .trim()
        .to_owned()
}

fn socket_probe(family: &str, kind: &str) -> String {
    format!(
        "import socket\ntry:\n s = socket.socket(socket.{family}, socket.{kind})\n\
         except OSError as e:\n print(f'errno={{e.errno}}')\nelse:\n s.close()\n print('ok')\n"
    )
}

/// `syscall(number, args)` through ctypes, bypassing Python's socket
/// module; `args` is a Python expression list of ctypes values (`buf` is a
/// zeroed 120-byte `io_uring_params`).
fn raw_probe(number: i64, args: &str) -> String {
    format!(
        "import ctypes, os\nlibc = ctypes.CDLL(None, use_errno=True)\n\
         buf = ctypes.create_string_buffer(120)\n\
         rc = libc.syscall(ctypes.c_long({number}), {args})\n\
         print('ok' if rc >= 0 else f'errno={{ctypes.get_errno()}}')\n\
         rc >= 0 and os.close(rc)\n"
    )
}

/// AF_INET (2) with garbage in the upper half of the register.
fn high_bit_inet_args() -> String {
    format!(
        "ctypes.c_long({}), ctypes.c_long(1), ctypes.c_long(0)",
        (1u64 << 32) | 2
    )
}

const URING_ARGS: &str = "ctypes.c_long(1), buf";

// --- validation -------------------------------------------------------------

#[test]
fn unfiltered_control_creates_inet_sockets() {
    assert_eq!(probe(&socket_probe("AF_INET", "SOCK_STREAM"), false), "ok");
    assert_eq!(probe(&socket_probe("AF_INET6", "SOCK_STREAM"), false), "ok");
}

#[test]
fn filtered_child_runs_ordinary_code() {
    let script = "print('ok' if sum(range(10)) == 45 else 'bad')";
    assert_eq!(probe(script, true), "ok");
}

#[test]
fn filtered_child_keeps_unix_sockets() {
    assert_eq!(probe(&socket_probe("AF_UNIX", "SOCK_STREAM"), true), "ok");
    assert_eq!(probe(&socket_probe("AF_UNIX", "SOCK_DGRAM"), true), "ok");
}

#[test]
fn filtered_child_has_no_new_privs_and_a_filter() {
    // An outer sandbox may already have set both, so compare against an
    // unfiltered control: exactly one more filter, and NoNewPrivs set.
    let script = "s = dict(l.split(':\\t', 1) for l in open('/proc/self/status').read() \
                  .splitlines() if ':\\t' in l)\n\
                  print(s['NoNewPrivs'], s['Seccomp'], s['Seccomp_filters'])";
    let fields = |line: String| -> Vec<u32> {
        line.split(' ')
            .map(|field| field.parse().expect("numeric status field"))
            .collect()
    };
    let control = fields(probe(script, false));
    let filtered = fields(probe(script, true));
    assert_eq!(filtered[0], 1, "NoNewPrivs");
    assert_eq!(filtered[1], 2, "Seccomp mode: filter");
    assert_eq!(filtered[2], control[2] + 1, "Seccomp_filters");
}

#[test]
fn unfiltered_control_allows_io_uring_and_high_bit_family() {
    let high = high_bit_inet_args();
    assert_eq!(probe(&raw_probe(libc::SYS_socket, &high), false), "ok");
    let uring = raw_probe(libc::SYS_io_uring_setup, URING_ARGS);
    assert_eq!(probe(&uring, false), "ok");
}

// --- adversarial ------------------------------------------------------------

#[test]
fn inet_stream_sockets_are_eperm() {
    assert_eq!(probe(&socket_probe("AF_INET", "SOCK_STREAM"), true), EPERM);
    assert_eq!(probe(&socket_probe("AF_INET6", "SOCK_STREAM"), true), EPERM);
}

#[test]
fn inet_datagram_sockets_are_eperm() {
    assert_eq!(probe(&socket_probe("AF_INET", "SOCK_DGRAM"), true), EPERM);
    assert_eq!(probe(&socket_probe("AF_INET6", "SOCK_DGRAM"), true), EPERM);
}

#[test]
fn upper_register_bits_do_not_bypass_the_family_check() {
    // The kernel truncates the family to int, so this is AF_INET.
    let high = high_bit_inet_args();
    assert_eq!(probe(&raw_probe(libc::SYS_socket, &high), true), EPERM);
}

#[test]
fn io_uring_setup_is_eperm() {
    let uring = raw_probe(libc::SYS_io_uring_setup, URING_ARGS);
    assert_eq!(probe(&uring, true), EPERM);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x32_syscall_aliases_are_eperm() {
    let inet = "ctypes.c_long(2), ctypes.c_long(1), ctypes.c_long(0)";
    let socket = raw_probe(libc::SYS_socket | X32_SYSCALL_BIT, inet);
    assert_eq!(probe(&socket, true), EPERM);
    let uring = raw_probe(libc::SYS_io_uring_setup | X32_SYSCALL_BIT, URING_ARGS);
    assert_eq!(probe(&uring, true), EPERM);
}

#[test]
fn grandchildren_inherit_the_filter_across_exec() {
    let inner = socket_probe("AF_INET", "SOCK_STREAM");
    let script = format!(
        "import subprocess, sys\n\
         sys.stdout.write(subprocess.run(['/bin/sh', '-c', 'exec \"$0\" -c \"$1\"', \
         '{PYTHON}', {inner:?}], capture_output=True, text=True).stdout)"
    );
    assert_eq!(probe(&script, true), EPERM);
}

#[test]
fn filtered_child_cannot_connect_to_a_live_listener() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("host loopback listener");
    let port = listener.local_addr().expect("listener address").port();
    let script = format!(
        "import socket\ntry:\n socket.create_connection(('127.0.0.1', {port}), 1).close()\n\
         except OSError as e:\n print(f'errno={{e.errno}}')\nelse:\n print('ok')\n"
    );
    assert_eq!(probe(&script, false), "ok");
    assert_eq!(probe(&script, true), EPERM);
}
