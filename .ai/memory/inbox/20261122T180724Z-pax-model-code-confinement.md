# Model-code child confinement

Trainlab (including autoresearch run-live rewards) and self-improve proposed
check validation enter `phlow_seccomp::confined_command` before execution.
Ordinary operator-approved checks retain their egress-only entry point.

Primo returns 11 for the Landlock version syscall. The policy requires ABI 9
and unprivileged user, mount, network and IPC namespace setup. The synthetic
root exposes the Arch system runtime read-only, plus either the staged
trainlab work directory or a read-only revision and tmpfs scratch. It exposes
no procfs or ordinary home path. The egress filter runs first and setup failure
returns a spawn error before the workload starts. Descriptor sealing keeps
only standard I/O across exec.

On primo, a nonrecursive `/usr/lib` bind fails with EINVAL because its mkmm
submount becomes locked across the user namespace boundary. The system runtime
binds are recursive and are made recursively read-only. Input and work binds
are nonrecursive. A dirfd opened before unshare also cannot be used directly as
a bind source in the new namespace. Pin and verify the cwd identity, then bind
relative to that cwd after unshare.

Validation: `TMPDIR=/tmp cargo test -p phlow-seccomp -p phlow-trainlab -p phlow-self-improve -p phlow-autoresearch --no-fail-fast`
passed 251 tests on primo, including the independent Landlock denial and
required filesystem probes in `crates/phlow-seccomp/tests/filesystem.rs`.
