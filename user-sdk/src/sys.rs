// SPDX-License-Identifier: GPL-3.0-only
//! Thin, safe wrappers over the provisional n32 syscalls.
//!
//! Every result is mapped to Rust types without inventing an `errno` value.
//! The bring-up kernel reports failure as `-1` (`usize::MAX`) in `$v0` and has
//! no per-thread error state, so the wrappers return `-1`, `None` or `false`
//! rather than a POSIX error number.  This is deliberate: POSIX error handling
//! belongs to relibc, not to this runtime.

use crate::arch::{syscall0, syscall1, syscall2, syscall3, syscall4};
use user_abi::*;

/// Longest path accepted by the bring-up kernel.
pub const PATH_MAX: usize = 64;

/// Convert a raw `$v0` result to an `isize`.
fn result(value: usize) -> isize {
    if value == usize::MAX {
        -1
    } else {
        value as isize
    }
}

/// Copy `path` into a NUL-terminated stack buffer and pass it to `call`.
fn with_path<T>(path: &[u8], call: impl FnOnce(*const u8) -> T) -> Option<T> {
    if path.is_empty() || path.len() >= PATH_MAX {
        return None;
    }
    let mut buffer = [0u8; PATH_MAX];
    buffer[..path.len()].copy_from_slice(path);
    Some(call(buffer.as_ptr()))
}

/// Write up to `buffer.len()` bytes to `fd`.
///
/// Returns the number of bytes written, or `-1`.
pub fn write(fd: i32, buffer: &[u8]) -> isize {
    // SAFETY: the kernel validates the user range before copying.
    unsafe {
        result(syscall3(
            SYS_WRITE,
            fd as usize,
            buffer.as_ptr() as usize,
            buffer.len(),
        ))
    }
}

/// Read up to `buffer.len()` bytes from `fd`.
///
/// Returns the number of bytes read, `0` at end of file, or `-1`.
pub fn read(fd: i32, buffer: &mut [u8]) -> isize {
    // SAFETY: the kernel validates the user range before copying.
    unsafe {
        result(syscall3(
            SYS_READ,
            fd as usize,
            buffer.as_mut_ptr() as usize,
            buffer.len(),
        ))
    }
}

/// Open a path in the bring-up namespace.
///
/// Only `O_TRUNC` is acted on.  The namespace contains `/`, `/etc`, `/dev`,
/// `/dev/null`, `/dev/tty`, `/hello.txt` and `/etc/motd`.
pub fn open(path: &[u8], flags: u32) -> i32 {
    match with_path(path, |pointer| {
        // SAFETY: the pointer is a NUL-terminated stack buffer.
        unsafe { syscall3(SYS_OPEN, pointer as usize, flags as usize, 0) }
    }) {
        Some(value) if value != usize::MAX => value as i32,
        _ => -1,
    }
}

/// Close a descriptor.
pub fn close(fd: i32) -> bool {
    // SAFETY: a descriptor number needs no pointer validation.
    unsafe { syscall1(SYS_CLOSE, fd as usize) != usize::MAX }
}

/// Duplicate a descriptor.
pub fn dup(fd: i32) -> i32 {
    // SAFETY: a descriptor number needs no pointer validation.
    let value = unsafe { syscall1(SYS_DUP, fd as usize) };
    if value == usize::MAX {
        -1
    } else {
        value as i32
    }
}

/// Descriptor control; the bring-up kernel ignores the command.
pub fn fcntl(fd: i32, command: i32, argument: i32) -> i32 {
    // SAFETY: a descriptor number and scalar arguments need no validation.
    let value = unsafe { syscall3(SYS_FCNTL, fd as usize, command as usize, argument as usize) };
    if value == usize::MAX {
        -1
    } else {
        value as i32
    }
}

/// Read the current working directory into `buffer` and return it without the
/// NUL terminator.
pub fn getcwd(buffer: &mut [u8]) -> Option<&[u8]> {
    if buffer.is_empty() {
        return None;
    }
    // SAFETY: the kernel validates the user range before copying.
    let value = unsafe { syscall2(SYS_GETCWD, buffer.as_mut_ptr() as usize, buffer.len()) };
    if value == usize::MAX {
        return None;
    }
    let length = buffer.iter().position(|byte| *byte == 0)?;
    Some(&buffer[..length])
}

/// Change the current working directory.
pub fn chdir(path: &[u8]) -> bool {
    with_path(path, |pointer| {
        // SAFETY: the pointer is a NUL-terminated stack buffer.
        unsafe { syscall1(SYS_CHDIR, pointer as usize) != usize::MAX }
    })
    .unwrap_or(false)
}

/// Read one directory entry by index into `name`.
pub fn read_dir(fd: i32, index: usize, name: &mut [u8]) -> ReadDir {
    if name.is_empty() {
        return ReadDir::Error;
    }
    // SAFETY: the kernel validates the user range before copying.
    let value = unsafe {
        syscall4(
            SYS_READDIR,
            fd as usize,
            index,
            name.as_mut_ptr() as usize,
            name.len(),
        )
    };
    match value {
        0 => ReadDir::End,
        usize::MAX => ReadDir::Error,
        kind => {
            let length = name.iter().position(|byte| *byte == 0).unwrap_or(0);
            ReadDir::Entry {
                kind: kind as u8,
                length,
            }
        }
    }
}

/// Result of [`read_dir`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadDir {
    /// A directory entry of `kind` with a name of `length` bytes.
    Entry {
        /// One of the `DT_*` constants.
        kind: u8,
        /// Length of the name written into the caller's buffer.
        length: usize,
    },
    /// The end of the directory.
    End,
    /// The descriptor is not a directory, or a buffer failed validation.
    Error,
}

/// Stat an open descriptor.
pub fn fstat(fd: i32, stat: &mut Stat) -> bool {
    // SAFETY: the kernel validates the user range before copying.
    unsafe { syscall2(SYS_FSTAT, fd as usize, stat as *mut Stat as usize) != usize::MAX }
}

/// Stat a path.
pub fn stat(path: &[u8], stat: &mut Stat) -> bool {
    with_path(path, |pointer| {
        // SAFETY: both pointers are valid for the duration of the call.
        unsafe { syscall2(SYS_STAT, pointer as usize, stat as *mut Stat as usize) != usize::MAX }
    })
    .unwrap_or(false)
}

/// Read a clock into a [`Timespec`].
pub fn clock_gettime(clock: i32) -> Option<Timespec> {
    let mut timespec = Timespec::default();
    // SAFETY: the kernel validates the user range before copying.
    let value = unsafe {
        syscall2(
            SYS_CLOCK_GETTIME,
            clock as usize,
            &mut timespec as *mut Timespec as usize,
        )
    };
    if value == usize::MAX {
        None
    } else {
        Some(timespec)
    }
}

/// Sleep for `request`.  The bring-up kernel busy-waits on CP0 Count.
pub fn nanosleep(request: &Timespec) -> bool {
    // SAFETY: the kernel validates the user range before reading.
    unsafe { syscall1(SYS_NANOSLEEP, request as *const Timespec as usize) != usize::MAX }
}

/// Map zero-filled anonymous pages and return the base address.
pub fn mmap(length: usize) -> Option<u32> {
    // SAFETY: no pointer is passed; the kernel ignores address and protection.
    let value = unsafe { syscall2(SYS_MMAP, 0, length) };
    if value == 0 || value == usize::MAX {
        None
    } else {
        Some(value as u32)
    }
}

/// Release a mapping.  The bring-up heap is one-way, so this always reports
/// success and does not return pages.
pub fn munmap(_address: u32, _length: usize) -> bool {
    // SAFETY: no pointer is dereferenced by the kernel.
    unsafe { syscall2(SYS_MUNMAP, _address as usize, _length) != usize::MAX }
}

/// Change mapping permissions.  Every bring-up user page is read/write, so this
/// always reports success.
pub fn mprotect(_address: u32, _length: usize, _protection: i32) -> bool {
    // SAFETY: no pointer is dereferenced by the kernel.
    unsafe {
        syscall3(
            SYS_MPROTECT,
            _address as usize,
            _length,
            _protection as usize,
        ) != usize::MAX
    }
}

/// Fill `buffer` with entropy.  The bring-up kernel writes zeros.
pub fn getentropy(buffer: &mut [u8]) -> bool {
    // SAFETY: the kernel validates the user range before copying.
    unsafe { syscall2(SYS_GETENTROPY, buffer.as_mut_ptr() as usize, buffer.len()) != usize::MAX }
}

/// Poll descriptors.  Returns the number of ready descriptors or `-1`.
pub fn poll(descriptors: &mut [PollFd], timeout_ms: i32) -> i32 {
    // SAFETY: the kernel validates the user range before reading or writing.
    let value = unsafe {
        syscall3(
            SYS_POLL,
            descriptors.as_mut_ptr() as usize,
            descriptors.len(),
            timeout_ms as usize,
        )
    };
    if value == usize::MAX {
        -1
    } else {
        value as i32
    }
}

/// Signal registration.  The bring-up kernel has no signal delivery; the call
/// reports success without installing a handler.
pub fn signal(signal: i32, handler: usize) -> usize {
    // SAFETY: a scalar handler value needs no pointer validation.
    unsafe { syscall2(SYS_SIGNAL, signal as usize, handler) }
}

/// Return the process id.  The bring-up kernel has one process, so this is
/// always `1`.
pub fn getpid() -> i32 {
    // SAFETY: no argument is passed.
    unsafe { syscall0(SYS_GETPID) as i32 }
}
