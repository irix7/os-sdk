// SPDX-License-Identifier: GPL-3.0-only
//! Byte and string primitives emitted by the compiler.
//!
//! `build-std=core` links `compiler_builtins`, but not every mem primitive the
//! optimiser can call is present in that configuration.  The module SDK has the
//! same set; the loops use volatile accesses so LLVM cannot turn them back into
//! calls to themselves.

use core::ffi::c_void;

/// Fill `n` bytes at `destination` with `value`.
///
/// # Safety
/// `destination` must be valid for writes of `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut c_void, value: i32, n: usize) -> *mut c_void {
    let bytes = destination as *mut u8;
    for index in 0..n {
        // SAFETY: the caller guarantees the range.
        unsafe {
            core::ptr::write_volatile(bytes.add(index), value as u8);
        }
    }
    destination
}

/// Copy `n` bytes from `source` to `destination`.
///
/// # Safety
/// Both ranges must be valid for `n` bytes and must not overlap.
#[no_mangle]
pub unsafe extern "C" fn memcpy(
    destination: *mut c_void,
    source: *const c_void,
    n: usize,
) -> *mut c_void {
    let destination = destination as *mut u8;
    let source = source as *const u8;
    for index in 0..n {
        // SAFETY: the caller guarantees both ranges.
        unsafe {
            core::ptr::write_volatile(
                destination.add(index),
                core::ptr::read_volatile(source.add(index)),
            );
        }
    }
    destination as *mut c_void
}

/// Copy `n` bytes, allowing overlap.
///
/// # Safety
/// Both ranges must be valid for `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn memmove(
    destination: *mut c_void,
    source: *const c_void,
    n: usize,
) -> *mut c_void {
    let destination = destination as *mut u8;
    let source = source as *const u8;
    if (destination as usize) <= (source as usize) {
        for index in 0..n {
            // SAFETY: the caller guarantees both ranges.
            unsafe {
                core::ptr::write_volatile(
                    destination.add(index),
                    core::ptr::read_volatile(source.add(index)),
                );
            }
        }
    } else {
        for index in (0..n).rev() {
            // SAFETY: the caller guarantees both ranges.
            unsafe {
                core::ptr::write_volatile(
                    destination.add(index),
                    core::ptr::read_volatile(source.add(index)),
                );
            }
        }
    }
    destination as *mut c_void
}

/// Compare `n` bytes.
///
/// # Safety
/// Both ranges must be valid for reads of `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const c_void, right: *const c_void, n: usize) -> i32 {
    let left = left as *const u8;
    let right = right as *const u8;
    for index in 0..n {
        // SAFETY: the caller guarantees both ranges.
        let a = unsafe { core::ptr::read_volatile(left.add(index)) };
        // SAFETY: the caller guarantees both ranges.
        let b = unsafe { core::ptr::read_volatile(right.add(index)) };
        if a != b {
            return a as i32 - b as i32;
        }
    }
    0
}

/// `bcmp` is the POSIX spelling of a comparison that only tests equality.
///
/// # Safety
/// Both ranges must be valid for reads of `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn bcmp(left: *const c_void, right: *const c_void, n: usize) -> i32 {
    // SAFETY: the caller's contract is the same as memcmp's.
    unsafe { memcmp(left, right, n) }
}
