// SPDX-License-Identifier: GPL-3.0-only
//! Example module: prints the SDK greeting and the live machine status.

#![no_std]
#![no_main]

use module_sdk::{Context, ModuleVtable};

static VTABLE: ModuleVtable = ModuleVtable {
    status: 0,
    tick: None,
};

fn run(context: &Context) -> *const ModuleVtable {
    context.println("hello from the IRIX 7 module SDK");
    let mut buf = [0u8; 512];
    context.write(context.info(0, &mut buf));
    &VTABLE
}

module_sdk::entry!(run);
module_sdk::panic_handler!();
