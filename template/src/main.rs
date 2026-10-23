// SPDX-License-Identifier: GPL-3.0-only
//! Template module: prints a line and returns a one-shot vtable.

#![no_std]
#![no_main]

use module_sdk::{Context, ModuleVtable};

static VTABLE: ModuleVtable = ModuleVtable {
    status: 0,
    tick: None,
};

fn run(context: &Context) -> *const ModuleVtable {
    context.println("hello from my IRIX 7 module");
    &VTABLE
}

module_sdk::entry!(run);
module_sdk::panic_handler!();
