// SPDX-License-Identifier: GPL-3.0-only
//! Argument and environment access from the initial stack.
//!
//! The loader builds the conventional layout:
//!
//! ```text
//! sp + 0:                      argc
//! sp + 4:                      argv[0] ... argv[argc-1], NULL
//! sp + 8 + 4 * argc:           envp[0] ... envp[n-1], NULL
//! above the pointer arrays:    NUL-terminated strings
//! ```
//!
//! Strings are byte slices: IRIX paths and arguments are not required to be
//! UTF-8.  The references are valid for the lifetime of the image because the
//! kernel stack is not reused while the image runs.  This is a single-threaded
//! bring-up runtime; there is no process table and no `exec`.

use crate::Context;

/// Maximum number of arguments accepted from the stack.
pub const MAX_ARGS: usize = 4096;
/// Maximum number of environment strings accepted from the stack.
pub const MAX_ENV: usize = 256;
/// Maximum length of one argument or environment string.
pub const MAX_STRING: usize = 4096;

/// Iterator over the argument vector.
#[derive(Clone, Copy)]
pub struct Args<'a> {
    context: &'a Context,
    index: usize,
}

impl<'a> Args<'a> {
    pub(crate) fn new(context: &'a Context) -> Self {
        Self { context, index: 0 }
    }
}

impl Iterator for Args<'_> {
    type Item = &'static [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.context.arg(self.index)?;
        self.index += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.context.argc().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Args<'_> {}

/// Iterator over the environment vector.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    context: &'a Context,
    index: usize,
}

impl<'a> Env<'a> {
    pub(crate) fn new(context: &'a Context) -> Self {
        Self { context, index: 0 }
    }
}

impl Iterator for Env<'_> {
    type Item = &'static [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.context.env_at(self.index)?;
        self.index += 1;
        Some(value)
    }
}

/// Read a NUL-terminated string from user memory.
///
/// # Safety
/// `pointer` must point into the initial user stack, or be null.
pub(crate) unsafe fn read_c_string(pointer: *const u8) -> Option<&'static [u8]> {
    if pointer.is_null() {
        return None;
    }
    let mut length = 0;
    while length < MAX_STRING {
        // SAFETY: the caller guarantees the pointer is in the user stack.
        let byte = unsafe { core::ptr::read_volatile(pointer.add(length)) };
        if byte == 0 {
            // SAFETY: the bytes are inside the kernel-owned initial stack and
            // stay mapped until the image exits.
            return Some(unsafe { core::slice::from_raw_parts(pointer, length) });
        }
        length += 1;
    }
    None
}
