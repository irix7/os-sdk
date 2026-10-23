// SPDX-License-Identifier: GPL-3.0-only
//! Block example: reads block 0 of SCSI target 1 through the ABI 10
//! `block_read` service and prints the volume-header magic.

#![no_std]
#![no_main]

use module_sdk::{Context, ModuleVtable, Services};

/// Oldest service table with `block_read` at offset `0x4c`.
const REQUIRED_ABI: u32 = 10;

/// SCSI target of the system disk, and the LBA of the volume header.
const TARGET: u8 = 1;
const VOLUME_HEADER_LBA: u32 = 0;

const MAGIC_PREFIX: &[u8] = b"block: magic 0x";
const HEX: &[u8; 16] = b"0123456789abcdef";

static OK: ModuleVtable = ModuleVtable {
    status: 0,
    tick: None,
};

static FAILED: ModuleVtable = ModuleVtable {
    status: 1,
    tick: None,
};

fn run(context: &Context) -> *const ModuleVtable {
    let mut sector = [0u8; 512];
    if !context.block_read(TARGET, VOLUME_HEADER_LBA, &mut sector) {
        context.println("block: read failed");
        return &FAILED;
    }

    let magic = u32::from_be_bytes([sector[0], sector[1], sector[2], sector[3]]);
    let mut line = [0u8; MAGIC_PREFIX.len() + 8];
    line[..MAGIC_PREFIX.len()].copy_from_slice(MAGIC_PREFIX);
    for (index, byte) in magic.to_be_bytes().iter().enumerate() {
        line[MAGIC_PREFIX.len() + index * 2] = HEX[(byte >> 4) as usize];
        line[MAGIC_PREFIX.len() + index * 2 + 1] = HEX[(byte & 0x0f) as usize];
    }
    context.write(&line);
    context.write(b"\r\n");
    &OK
}

/// # Safety
/// `services` must point to a valid kernel service table for this module.
#[no_mangle]
pub unsafe extern "C" fn _start(services: *const Services) -> *const ModuleVtable {
    let Some(table) = (unsafe { services.as_ref() }) else {
        return &FAILED;
    };
    if table.version < REQUIRED_ABI {
        // `write` exists in every table, so the message stays bounded even
        // when the table predates `block_read`.
        let message = b"block: ABI 10 required\r\n";
        (table.write)(message.as_ptr(), message.len());
        return &FAILED;
    }
    match unsafe { Context::new(services) } {
        Some(context) => run(&context),
        None => &FAILED,
    }
}

module_sdk::panic_handler!();
