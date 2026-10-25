// SPDX-License-Identifier: GPL-3.0-only
//! Example static n32 user image: console, arguments, environment and BSS.
//!
//! It exercises the SDK runtime, not POSIX: `println!` writes to descriptor 1,
//! `Context` reads the loader-built stack, and the exit syscall returns the
//! status to the console.  The `.bss` guard catches a loader that forgets to
//! zero the memory range beyond `p_filesz`.

#![no_std]
#![no_main]

use user_sdk::{entry, panic_handler, println, Context};

/// Forces the linker to emit a `.bss` section larger than the file image.
static mut BSS_GUARD: [u8; 8192] = [0; 8192];

fn run(context: &Context) -> i32 {
    println!("hello from the IRIX 7 user SDK");
    println!("argc={}", context.argc());
    for (index, argument) in context.args().enumerate() {
        let text = core::str::from_utf8(argument).unwrap_or("<non-utf8>");
        println!("argv[{}]={}", index, text);
    }
    for name in [b"PATH".as_slice(), b"HOME".as_slice()] {
        match context.env(name) {
            Some(value) => {
                let text = core::str::from_utf8(value).unwrap_or("<non-utf8>");
                println!("env {}={}", core::str::from_utf8(name).unwrap_or("?"), text);
            }
            None => println!("env missing"),
        }
    }

    // SAFETY: the address is a static byte; only this image reads it.
    let first = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(BSS_GUARD[0])) };
    // SAFETY: the address is a static byte; only this image reads it.
    let last = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(BSS_GUARD[8191])) };
    if first != 0 || last != 0 {
        println!("bss: not zero");
        return 1;
    }
    println!("bss: zeroed");
    println!("exit=0");
    0
}

entry!(run);
panic_handler!();
