// SPDX-License-Identifier: GPL-3.0-only
//! Runtime for static native n32 IRIX 7 user images.
//!
//! This crate is a **bring-up runtime**, not a libc and not a POSIX
//! implementation.  It provides the smallest useful surface for a `no_std`
//! program that the current kernel loader can start:
//!
//! - [`entry!`] defines `__irix7_user_main`; the runtime supplies `_start`.
//! - [`Context`] exposes the argument vector, environment, console, files and
//!   clocks.
//! - [`sys`] wraps the provisional syscall numbers in [`user_abi`].
//! - [`print!`] and [`println!`] format through `core::fmt` to descriptor 1.
//!
//! The image is linked at `0x1000_0000`, is a static `ET_EXEC` ELF with no
//! interpreter and no relocations, and is loaded from LBA 0 of SCSI target 2.
//! The loader reads at most 256 KiB, so `tools/irix7-user.py` pads the image to
//! exactly 262144 bytes.
//!
//! # What this is not
//!
//! - **Not POSIX.**  There is no `errno`, thread-local storage, process table,
//!   `fork`, `exec`, signal delivery, `termios`, sockets or general filesystem.
//! - **Not the final libc.**  The production POSIX C layer is the separate
//!   [`irix7/relibc`](https://github.com/irix7/relibc) fork on `irix7-n32`.
//! - **Not a kernel module.**  Modules are flat kernel-mode images at
//!   `0x8840_0000`; this runtime produces a user-mode ELF at `0x1000_0000`.
//!
//! # Example
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use user_sdk::Context;
//!
//! fn run(context: &Context) -> i32 {
//!     context.println("hello from userland");
//!     0
//! }
//!
//! user_sdk::entry!(run);
//! user_sdk::panic_handler!();
//! ```

#![no_std]
#![cfg_attr(target_arch = "mips64", feature(asm_experimental_arch))]
#![deny(missing_docs)]

use core::ptr::read_volatile;

mod arch;
pub mod args;
pub mod console;
pub mod fs;
mod mem;
mod rt;
pub mod sys;
pub mod time;

pub use args::{Args, Env};
pub use rt::{abort, exit};
pub use user_abi as abi;
pub use user_abi::{
    PollFd, Stat, Timespec, CLOCK_MONOTONIC, CLOCK_REALTIME, O_APPEND, O_CREAT, O_RDONLY, O_RDWR,
    O_TRUNC, O_WRONLY,
};

use args::{MAX_ARGS, MAX_ENV};

/// The state the loader handed to the image.
///
/// A `Context` borrows the kernel-built initial stack and is valid until the
/// image exits.  It is cheap to copy; the iterators borrow it.
#[derive(Clone, Copy)]
pub struct Context {
    stack: *const u32,
    argc: usize,
    argv: *const u32,
    envp: *const u32,
}

impl Context {
    /// Validate the initial stack and build a context.
    ///
    /// The loader passes the stack pointer in `$sp` with `argc` at offset 0,
    /// `argv` at offset 4 and `envp` after the `argv` NULL terminator.
    ///
    /// # Safety
    /// `stack` must be the stack pointer supplied to `_start` by the IRIX 7
    /// loader.  The function reads the pointed-to words and NUL-terminated
    /// strings, but cannot prove the range is mapped.
    pub unsafe fn from_stack(stack: *const u32) -> Option<Self> {
        if stack.is_null() || (stack as usize) > u32::MAX as usize || (stack as usize) & 3 != 0 {
            return None;
        }
        // SAFETY: the caller guarantees the loader's initial stack.
        let argc = unsafe { read_volatile(stack) } as usize;
        if argc > MAX_ARGS {
            return None;
        }
        // SAFETY: argv starts immediately after argc.
        let argv = unsafe { stack.add(1) };
        for index in 0..argc {
            // SAFETY: the loader writes argc pointers followed by a NULL.
            let pointer = unsafe { read_volatile(argv.add(index)) };
            if pointer == 0 || unsafe { args::read_c_string(pointer as *const u8) }.is_none() {
                return None;
            }
        }
        // SAFETY: argv[argc] is the NULL terminator.
        if unsafe { read_volatile(argv.add(argc)) } != 0 {
            return None;
        }
        // SAFETY: envp starts after the argv NULL terminator.
        let envp = unsafe { argv.add(argc + 1) };
        for index in 0..MAX_ENV {
            // SAFETY: the loader writes a NULL-terminated environment vector.
            let pointer = unsafe { read_volatile(envp.add(index)) };
            if pointer == 0 {
                break;
            }
            if unsafe { args::read_c_string(pointer as *const u8) }.is_none()
                || index + 1 == MAX_ENV
            {
                return None;
            }
        }
        Some(Self {
            stack,
            argc,
            argv,
            envp,
        })
    }

    /// The initial stack pointer.
    pub fn stack_pointer(&self) -> *const u32 {
        self.stack
    }

    /// Number of arguments, including `argv[0]`.
    pub fn argc(&self) -> usize {
        self.argc
    }

    /// Iterate over the argument vector.
    pub fn args(&self) -> Args<'_> {
        Args::new(self)
    }

    /// Iterate over the environment vector.
    pub fn env_vars(&self) -> Env<'_> {
        Env::new(self)
    }

    /// Return one argument, without its NUL terminator.
    pub fn arg(&self, index: usize) -> Option<&'static [u8]> {
        if index >= self.argc {
            return None;
        }
        // SAFETY: index is inside the validated argv array.
        let pointer = unsafe { read_volatile(self.argv.add(index)) };
        // SAFETY: from_stack validated every argv string.
        unsafe { args::read_c_string(pointer as *const u8) }
    }

    /// Return one environment string by index.
    pub fn env_at(&self, index: usize) -> Option<&'static [u8]> {
        if index >= MAX_ENV {
            return None;
        }
        // SAFETY: index is below the cap checked by from_stack.
        let pointer = unsafe { read_volatile(self.envp.add(index)) };
        if pointer == 0 {
            return None;
        }
        // SAFETY: from_stack validated every envp string up to the cap.
        unsafe { args::read_c_string(pointer as *const u8) }
    }

    /// Look up `name` in the environment and return the value after `=`.
    pub fn env(&self, name: &[u8]) -> Option<&'static [u8]> {
        for entry in self.env_vars() {
            let Some(separator) = entry.iter().position(|byte| *byte == b'=') else {
                continue;
            };
            if &entry[..separator] == name {
                return Some(&entry[separator + 1..]);
            }
        }
        None
    }

    /// Write bytes to the console.
    pub fn write(&self, bytes: &[u8]) -> isize {
        console::write(bytes)
    }

    /// Write a string to the console and append a newline.
    pub fn println(&self, text: &str) {
        console::write_all(text.as_bytes());
        console::write_all(b"\n");
    }

    /// Terminate the image and return `status` to the console.
    pub fn exit(&self, status: i32) -> ! {
        exit(status)
    }
}

/// Define the image entry point.
///
/// The supplied function has the signature `fn(&Context) -> i32`; its return
/// value is passed to the exit syscall.  The macro also defines the
/// `__irix7_user_main` symbol that the runtime's `_start` calls.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[no_mangle]
        pub extern "C" fn __irix7_user_main(stack: *const u32) -> i32 {
            match unsafe { $crate::Context::from_stack(stack) } {
                Some(context) => $main(&context),
                None => 127,
            }
        }
    };
}

/// Install the runtime's minimal panic handler.
///
/// A panic writes `user: panic` to the console and exits with status 134.  The
/// code is deliberately the C `abort` status, not a POSIX signal death.
#[macro_export]
macro_rules! panic_handler {
    () => {
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            $crate::console::write_all(b"user: panic\n");
            $crate::exit(134)
        }
    };
}
