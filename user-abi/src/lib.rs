// SPDX-License-Identifier: GPL-3.0-only
//! The provisional IRIX 7 native userland ABI.
//!
//! This crate is the SDK's single copy of the **bring-up** syscall numbers,
//! open flags and C layouts used by static n32 user images.  It mirrors the
//! kernel dispatch in `kernel/src/traps.rs` and the shim in
//! `crates/irix7-libc/src/sys.rs`; `tools/check-os-sync.py` compares the two
//! trees while the ABI is still provisional.
//!
//! This is **not** the native capability ABI in `docs/spec/native-abi.md` and
//! it is **not** a POSIX ABI.  POSIX userland is the separate
//! [`irix7/relibc`](https://github.com/irix7/relibc) fork.  When the native
//! capability ABI and relibc's IRIX 7 backend land, this crate is replaced
//! rather than extended.
//!
//! # ABI summary
//!
//! - n32, big-endian, MIPS III, soft float.
//! - Syscall number in `$v0`; arguments in `$a0`-`$a3`; result in `$v0`.
//! - `-1` (`usize::MAX`) is the only error value.  There is no `errno`
//!   syscall and no per-thread error state in the kernel.
//! - `Stat`, `Timespec` and `PollFd` are the exact `#[repr(C)]` layouts the
//!   kernel writes.

#![no_std]
#![deny(missing_docs)]

use core::mem::offset_of;

/// Version of the provisional userland ABI.
///
/// The kernel has no way to report this version at entry yet, so the value is
/// informational.  It is bumped when a syscall number or a structure layout
/// changes; it is not the module ABI version.
pub const PROVISIONAL_ABI_VERSION: u32 = 0;

/// Write bytes to a descriptor.
pub const SYS_WRITE: usize = 1;
/// Terminate the user image.
pub const SYS_EXIT: usize = 2;
/// Map zero-filled anonymous pages.
pub const SYS_MMAP: usize = 3;
/// Release a mapping (a no-op in the bring-up kernel).
pub const SYS_MUNMAP: usize = 4;
/// Change mapping permissions (a no-op in the bring-up kernel).
pub const SYS_MPROTECT: usize = 5;
/// Open a path in the bring-up namespace.
pub const SYS_OPEN: usize = 6;
/// Close a descriptor.
pub const SYS_CLOSE: usize = 7;
/// Descriptor control (accepted and ignored).
pub const SYS_FCNTL: usize = 8;
/// Duplicate a descriptor.
pub const SYS_DUP: usize = 9;
/// Read the current working directory.
pub const SYS_GETCWD: usize = 10;
/// Poll descriptors (console and null are always ready).
pub const SYS_POLL: usize = 11;
/// Signal registration (accepted and ignored; no signals yet).
pub const SYS_SIGNAL: usize = 12;
/// Fill a buffer with entropy (zero-filled in the bring-up kernel).
pub const SYS_GETENTROPY: usize = 13;
/// Read a clock into a [`Timespec`].
pub const SYS_CLOCK_GETTIME: usize = 14;
/// Return the process id (always 1 in the bring-up kernel).
pub const SYS_GETPID: usize = 15;
/// Read bytes from a descriptor.
pub const SYS_READ: usize = 16;
/// Stat an open descriptor.
pub const SYS_FSTAT: usize = 17;
/// Stat a path.
pub const SYS_STAT: usize = 18;
/// Read one directory entry by index.
pub const SYS_READDIR: usize = 19;
/// Sleep for a [`Timespec`] duration.
pub const SYS_NANOSLEEP: usize = 20;
/// Change the current working directory.
pub const SYS_CHDIR: usize = 21;

/// `O_RDONLY`.
pub const O_RDONLY: u32 = 0;
/// `O_WRONLY`; the bring-up descriptor table accepts it but does not enforce it.
pub const O_WRONLY: u32 = 1;
/// `O_RDWR`; the bring-up descriptor table accepts it but does not enforce it.
pub const O_RDWR: u32 = 2;
/// `O_CREAT`; accepted and ignored (the namespace is fixed).
pub const O_CREAT: u32 = 0o100;
/// `O_TRUNC`; the only flag the bring-up descriptor table acts on.
pub const O_TRUNC: u32 = 0o1000;
/// `O_APPEND`; accepted and ignored.
pub const O_APPEND: u32 = 0o2000;

/// Wall-clock clock id.
pub const CLOCK_REALTIME: i32 = 0;
/// Monotonic clock id.
pub const CLOCK_MONOTONIC: i32 = 1;

/// `POLLIN`.
pub const POLLIN: i16 = 0x001;
/// `POLLOUT`.
pub const POLLOUT: i16 = 0x004;
/// `POLLERR`.
pub const POLLERR: i16 = 0x008;
/// `POLLHUP`.
pub const POLLHUP: i16 = 0x010;
/// `POLLNVAL`; the bring-up kernel reports it for a closed descriptor.
pub const POLLNVAL: i16 = 0x020;

/// Directory-entry type for an unknown object.
pub const DT_UNKNOWN: u8 = 0;
/// Directory-entry type for a character device.
pub const DT_CHR: u8 = 2;
/// Directory-entry type for a directory.
pub const DT_DIR: u8 = 4;
/// Directory-entry type for a regular file.
pub const DT_REG: u8 = 8;

/// Mask for the file type bits of [`Stat::st_mode`].
pub const S_IFMT: u32 = 0o170000;
/// Regular file.
pub const S_IFREG: u32 = 0o100000;
/// Directory.
pub const S_IFDIR: u32 = 0o040000;
/// Character device.
pub const S_IFCHR: u32 = 0o020000;

/// The n32 `struct stat` written by the kernel.
///
/// The layout, including the `_pad` and `spare` fields, is part of the
/// provisional ABI.  Do not reorder fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Stat {
    /// Device number.
    pub st_dev: u64,
    /// Inode number.
    pub st_ino: u64,
    /// File type and mode bits.
    pub st_mode: u32,
    /// Link count.
    pub st_nlink: u32,
    /// Owner uid.
    pub st_uid: u32,
    /// Owner gid.
    pub st_gid: u32,
    /// Device number for a special file.
    pub st_rdev: u64,
    /// File size in bytes.
    pub st_size: i64,
    /// Preferred I/O block size.
    pub st_blksize: i32,
    /// Explicit padding.
    pub _pad: u32,
    /// Number of 512-byte blocks.
    pub st_blocks: i64,
    /// Access time: seconds and nanoseconds.
    pub st_atim: [i64; 2],
    /// Modification time: seconds and nanoseconds.
    pub st_mtim: [i64; 2],
    /// Change time: seconds and nanoseconds.
    pub st_ctim: [i64; 2],
    /// Reserved for the native capability ABI.
    pub spare: [u8; 16],
}

impl Stat {
    /// Whether this is a regular file.
    pub fn is_file(&self) -> bool {
        self.st_mode & S_IFMT == S_IFREG
    }

    /// Whether this is a directory.
    pub fn is_dir(&self) -> bool {
        self.st_mode & S_IFMT == S_IFDIR
    }

    /// Whether this is a character device.
    pub fn is_char_device(&self) -> bool {
        self.st_mode & S_IFMT == S_IFCHR
    }

    /// File size in bytes.
    pub fn size(&self) -> i64 {
        self.st_size
    }
}

/// The n32 `struct timespec` used by `clock_gettime` and `nanosleep`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    /// Whole seconds.
    pub tv_sec: i64,
    /// Nanoseconds within the second.
    pub tv_nsec: i32,
    /// Explicit padding to 16 bytes.
    pub _pad: u32,
}

impl Timespec {
    /// Construct a duration or timestamp from seconds and nanoseconds.
    pub const fn new(tv_sec: i64, tv_nsec: i32) -> Self {
        Self {
            tv_sec,
            tv_nsec,
            _pad: 0,
        }
    }

    /// Convert to whole milliseconds, saturating at zero.
    pub fn as_millis(&self) -> i64 {
        self.tv_sec
            .saturating_mul(1000)
            .saturating_add(self.tv_nsec as i64 / 1_000_000)
    }
}

/// The n32 `struct pollfd` used by `poll`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollFd {
    /// Descriptor to poll.
    pub fd: i32,
    /// Requested events.
    pub events: i16,
    /// Returned events.
    pub revents: i16,
}

// The n32 ABI fixes these sizes.  The layouts contain only fixed-width fields,
// so the assertions also hold on a 64-bit host and catch accidental edits
// during `cargo test`.
const _: () = {
    assert!(size_of::<Stat>() == 128);
    assert!(offset_of!(Stat, st_dev) == 0);
    assert!(offset_of!(Stat, st_ino) == 8);
    assert!(offset_of!(Stat, st_mode) == 16);
    assert!(offset_of!(Stat, st_nlink) == 20);
    assert!(offset_of!(Stat, st_uid) == 24);
    assert!(offset_of!(Stat, st_gid) == 28);
    assert!(offset_of!(Stat, st_rdev) == 32);
    assert!(offset_of!(Stat, st_size) == 40);
    assert!(offset_of!(Stat, st_blksize) == 48);
    assert!(offset_of!(Stat, _pad) == 52);
    assert!(offset_of!(Stat, st_blocks) == 56);
    assert!(offset_of!(Stat, st_atim) == 64);
    assert!(offset_of!(Stat, st_mtim) == 80);
    assert!(offset_of!(Stat, st_ctim) == 96);
    assert!(offset_of!(Stat, spare) == 112);
    assert!(size_of::<Timespec>() == 16);
    assert!(offset_of!(Timespec, tv_sec) == 0);
    assert!(offset_of!(Timespec, tv_nsec) == 8);
    assert!(size_of::<PollFd>() == 8);
    assert!(offset_of!(PollFd, fd) == 0);
    assert!(offset_of!(PollFd, events) == 4);
    assert!(offset_of!(PollFd, revents) == 6);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syscall_numbers_are_unique_and_contiguous() {
        let numbers = [
            SYS_WRITE,
            SYS_EXIT,
            SYS_MMAP,
            SYS_MUNMAP,
            SYS_MPROTECT,
            SYS_OPEN,
            SYS_CLOSE,
            SYS_FCNTL,
            SYS_DUP,
            SYS_GETCWD,
            SYS_POLL,
            SYS_SIGNAL,
            SYS_GETENTROPY,
            SYS_CLOCK_GETTIME,
            SYS_GETPID,
            SYS_READ,
            SYS_FSTAT,
            SYS_STAT,
            SYS_READDIR,
            SYS_NANOSLEEP,
            SYS_CHDIR,
        ];
        for (index, number) in numbers.iter().enumerate() {
            assert_eq!(*number, index + 1);
        }
    }

    #[test]
    fn file_types_are_classified() {
        let file = Stat {
            st_mode: S_IFREG | 0o644,
            ..Stat::default()
        };
        let directory = Stat {
            st_mode: S_IFDIR | 0o755,
            ..Stat::default()
        };
        let device = Stat {
            st_mode: S_IFCHR | 0o666,
            ..Stat::default()
        };
        assert!(file.is_file());
        assert!(directory.is_dir());
        assert!(device.is_char_device());
        assert_eq!(Stat::default().st_mode & S_IFMT, 0);
    }

    #[test]
    fn timespec_milliseconds_are_saturated() {
        assert_eq!(Timespec::new(1, 500_000_000).as_millis(), 1500);
        assert_eq!(Timespec::new(-1, 0).as_millis(), -1000);
        assert_eq!(Timespec::new(i64::MAX, 0).as_millis(), i64::MAX);
    }
}
