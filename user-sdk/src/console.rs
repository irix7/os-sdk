// SPDX-License-Identifier: GPL-3.0-only
//! Console output for a static user image.
//!
//! File descriptors 0, 1 and 2 are console descriptors in the bring-up kernel;
//! `write(1, ...)` reaches serial and telnet.  This is a console, not a POSIX
//! terminal: there is no `termios`, line discipline or input loop in the SDK.

use core::fmt::{self, Write};

/// Write bytes to the console.
///
/// Returns the number of bytes written, or `-1`.
pub fn write(bytes: &[u8]) -> isize {
    crate::sys::write(1, bytes)
}

/// Write all of `bytes`, retrying short writes.
///
/// Returns `false` if the kernel rejects the write or makes no progress.
pub fn write_all(bytes: &[u8]) -> bool {
    let mut offset = 0;
    while offset < bytes.len() {
        let written = write(&bytes[offset..]);
        if written <= 0 {
            return false;
        }
        offset += written as usize;
    }
    true
}

struct Writer;

impl Write for Writer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if write_all(text.as_bytes()) {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

/// Format into the console.
pub fn print(arguments: fmt::Arguments) {
    let _ = Writer.write_fmt(arguments);
}

/// Format a line into the console.
pub fn println(arguments: fmt::Arguments) {
    print(arguments);
    write_all(b"\n");
}

/// Write formatted text to the console.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::print(format_args!($($arg)*))
    };
}

/// Write formatted text and a newline to the console.
#[macro_export]
macro_rules! println {
    () => {{
        let _ = $crate::console::write_all(b"\n");
    }};
    ($($arg:tt)*) => {{
        $crate::console::print(format_args!($($arg)*));
        let _ = $crate::console::write_all(b"\n");
    }};
}
