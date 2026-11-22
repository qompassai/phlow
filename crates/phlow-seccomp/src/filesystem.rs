//! Mount and Landlock policy for model code. Allocate in the parent. Child
//! setup verifies the input directory and uses only libc syscall wrappers.

use std::ffi::CStr;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::RunDirectory;

const READ_EXECUTE: u64 = (1 << 0) | (1 << 2) | (1 << 3);
const HANDLED_FS: u64 = (1 << 17) - 1;
const MAKE_CHAR: u64 = 1 << 6;
const MAKE_BLOCK: u64 = 1 << 11;
const IOCTL_DEV: u64 = 1 << 15;
const RESOLVE_UNIX: u64 = 1 << 16;
const WORK_ACCESS: u64 = HANDLED_FS & !(MAKE_CHAR | MAKE_BLOCK | IOCTL_DEV | RESOLVE_UNIX);
const SCOPED_ABSTRACT_UNIX_AND_SIGNAL: u64 = 3;
const MINIMUM_ABI: libc::c_long = 9;
const MOUNT_SAFE: libc::c_ulong = libc::MS_NOSUID | libc::MS_NODEV;
const RUNTIME: [(&CStr, &CStr); 2] = [
    (c"/usr/bin", c"/tmp/usr/bin"),
    (c"/usr/lib", c"/tmp/usr/lib"),
];

pub(super) fn register(command: &mut Command, directory: RunDirectory<'_>) -> io::Result<()> {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0,
            1,
        )
    };
    check(abi)?;
    if abi < MINIMUM_ABI {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Landlock ABI 9 required",
        ));
    }
    let path = match directory {
        RunDirectory::Writable(path) | RunDirectory::ReadOnly(path) => path,
    };
    let source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
        .open(path)?;
    command.current_dir(path);
    let writable = matches!(directory, RunDirectory::Writable(_));
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let uid_map = format!("{uid} {uid} 1\n");
    let gid_map = format!("{gid} {gid} 1\n");

    // SAFETY: the closure keeps the source fd alive and only uses prebuilt
    // strings, stack data and async-signal-safe syscall wrappers after fork.
    unsafe {
        command.pre_exec(move || {
            verify_source(source.as_raw_fd())?;
            setup(writable, uid_map.as_bytes(), gid_map.as_bytes())
        });
    }
    Ok(())
}

// Command enters the requested cwd before pre_exec. Verify it is still
// the directory originally opened by the parent, then bind it by ".".
fn verify_source(source: libc::c_int) -> io::Result<()> {
    unsafe {
        let mut expected = std::mem::MaybeUninit::<libc::stat>::uninit();
        let mut actual = std::mem::MaybeUninit::<libc::stat>::uninit();
        check(libc::fstat(source, expected.as_mut_ptr()).into())?;
        check(libc::stat(c".".as_ptr(), actual.as_mut_ptr()).into())?;
        let expected = expected.assume_init();
        let actual = actual.assume_init();
        if (expected.st_dev, expected.st_ino) != (actual.st_dev, actual.st_ino) {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
    }
    Ok(())
}

fn check(result: libc::c_long) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn write_map(path: &CStr, value: &[u8]) -> io::Result<()> {
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        check(fd.into())?;
        let written = libc::write(fd, value.as_ptr().cast(), value.len());
        let result = check(written as libc::c_long);
        libc::close(fd);
        result?;
        if written as usize != value.len() {
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
    }
    Ok(())
}

fn mount(source: &CStr, target: &CStr, kind: &CStr, flags: libc::c_ulong) -> io::Result<()> {
    check(
        unsafe {
            libc::mount(
                source.as_ptr(),
                target.as_ptr(),
                kind.as_ptr(),
                flags,
                std::ptr::null(),
            )
        }
        .into(),
    )
}

fn bind(source: &CStr, target: &CStr, read_only: bool, recursive: bool) -> io::Result<()> {
    // Runtime binds include locked system submounts. Per-run binds must
    // never bring submounts in with them.
    let bind_flags = if recursive { libc::MS_REC } else { 0 };
    let attribute_flags = if recursive { libc::AT_RECURSIVE } else { 0 };
    mount(source, target, c"", libc::MS_BIND | bind_flags)?;
    let read_only = if read_only {
        libc::MOUNT_ATTR_RDONLY
    } else {
        0
    };
    // struct mount_attr: attr_set, attr_clr, propagation, userns_fd.
    let attributes = [
        read_only | libc::MOUNT_ATTR_NOSUID | libc::MOUNT_ATTR_NODEV,
        0,
        0,
        0,
    ];
    check(unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            libc::AT_FDCWD,
            target.as_ptr(),
            attribute_flags,
            attributes.as_ptr(),
            std::mem::size_of_val(&attributes),
        )
    })
}

fn setup(writable: bool, uid_map: &[u8], gid_map: &[u8]) -> io::Result<()> {
    unsafe {
        check(
            libc::unshare(
                libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWNET | libc::CLONE_NEWIPC,
            )
            .into(),
        )?;
    }
    write_map(c"/proc/self/setgroups", b"deny\n")?;
    write_map(c"/proc/self/uid_map", uid_map)?;
    write_map(c"/proc/self/gid_map", gid_map)?;
    mount(c"", c"/", c"", libc::MS_REC | libc::MS_PRIVATE)?;
    mount(c"tmpfs", c"/tmp", c"tmpfs", MOUNT_SAFE)?;
    unsafe {
        for dir in [
            c"/tmp/usr",
            c"/tmp/usr/bin",
            c"/tmp/usr/lib",
            c"/tmp/work",
            c"/tmp/input",
        ] {
            check(libc::mkdir(dir.as_ptr(), 0o755).into())?;
        }
        for (target, link) in [
            (c"usr/bin", c"/tmp/bin"),
            (c"usr/lib", c"/tmp/lib"),
            (c"usr/lib", c"/tmp/lib64"),
            (c"lib", c"/tmp/usr/lib64"),
        ] {
            check(libc::symlink(target.as_ptr(), link.as_ptr()).into())?;
        }
    }
    for (from, to) in RUNTIME {
        bind(from, to, true, true)?;
    }
    if writable {
        bind(c".", c"/tmp/work", false, false)?;
    } else {
        bind(c".", c"/tmp/input", true, false)?;
        mount(c"tmpfs", c"/tmp/work", c"tmpfs", MOUNT_SAFE)?;
    }
    mount(
        c"",
        c"/tmp",
        c"tmpfs",
        libc::MS_REMOUNT | libc::MS_RDONLY | MOUNT_SAFE,
    )?;
    enter_root(writable)
}

fn enter_root(writable: bool) -> io::Result<()> {
    unsafe {
        check(libc::chroot(c"/tmp".as_ptr()).into())?;
        let cwd = if writable { c"/work" } else { c"/input" };
        check(libc::chdir(cwd.as_ptr()).into())?;
        // Keep std's exec-error pipe functional until exec. All descriptors
        // above stderr, including ones supplied by the parent, close on exec.
        check(libc::syscall(
            libc::SYS_close_range,
            3u32,
            u32::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        ))?;
        // Linux capability API v3: clear effective, permitted and inheritable
        // sets. The egress hook already set no_new_privs.
        let header = [0x2008_0522u32, 0];
        let caps = [0u32; 6];
        check(libc::syscall(
            libc::SYS_capset,
            header.as_ptr(),
            caps.as_ptr(),
        ))?;
    }
    restrict(writable)
}

fn restrict(writable: bool) -> io::Result<()> {
    // Three u64 fields: handled_access_fs, handled_access_net, scoped.
    let attributes = [HANDLED_FS, 0, SCOPED_ABSTRACT_UNIX_AND_SIGNAL];
    let ruleset = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            attributes.as_ptr(),
            std::mem::size_of_val(&attributes),
            0,
        )
    };
    check(ruleset)?;
    let result = (|| {
        add_rule(ruleset, c"/usr/bin", READ_EXECUTE)?;
        add_rule(ruleset, c"/usr/lib", READ_EXECUTE)?;
        add_rule(ruleset, c"/work", WORK_ACCESS)?;
        if !writable {
            add_rule(ruleset, c"/input", READ_EXECUTE)?;
        }
        check(unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0) })
    })();
    unsafe { libc::close(ruleset as libc::c_int) };
    result
}

fn add_rule(ruleset: libc::c_long, path: &CStr, rights: u64) -> io::Result<()> {
    // Matches the packed Linux UAPI landlock_path_beneath_attr.
    #[repr(C, packed)]
    struct PathBeneath {
        allowed_access: u64,
        parent_fd: libc::c_int,
    }
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC);
        check(fd.into())?;
        let rule = PathBeneath {
            allowed_access: rights,
            parent_fd: fd,
        };
        let result = check(libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset,
            1,
            &rule,
        ));
        libc::close(fd);
        result
    }
}
