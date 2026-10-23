// SPDX-License-Identifier: GPL-3.0-only
//! Periodic module: registers a tick callback and prints once a second for
//! five seconds before unregistering.

#![no_std]
#![no_main]

use module_sdk::{Context, ModuleVtable, Services};

static VTABLE: ModuleVtable = ModuleVtable {
    status: 0,
    tick: Some(tick),
};

static mut LAST: u64 = 0;
static mut COUNT: u32 = 0;

fn run(context: &Context) -> *const ModuleVtable {
    context.println("ticker-sdk: registered");
    unsafe {
        LAST = context.clock_ms();
    }
    &VTABLE
}

unsafe extern "C" fn tick(services: *const Services) -> i32 {
    // SAFETY: the kernel passes the same valid service table on every call.
    let Some(context) = (unsafe { Context::new(services) }) else {
        return 1;
    };
    let now = context.clock_ms();
    let last = unsafe { LAST };
    if now.saturating_sub(last) < 1000 {
        return 0;
    }
    unsafe {
        LAST = now;
        COUNT += 1;
    }
    context.println("ticker-sdk: tick");
    if unsafe { COUNT } >= 5 {
        1
    } else {
        0
    }
}

module_sdk::entry!(run);
module_sdk::panic_handler!();
