// SPDX-License-Identifier: GPL-3.0-only
//! Template static n32 user program.
//!
//! Replace `run` with the program.  The return value becomes the exit status.
//! This is a bring-up runtime, not POSIX; see the SDK userland documentation.

#![no_std]
#![no_main]

use user_sdk::{entry, panic_handler, println, Context};

fn run(context: &Context) -> i32 {
    println!("hello from my IRIX 7 user program");
    println!("argc={}", context.argc());
    0
}

entry!(run);
panic_handler!();
