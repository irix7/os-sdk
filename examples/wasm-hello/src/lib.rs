// SPDX-License-Identifier: GPL-3.0-only
//! Minimal IRIX 7 Wasm guest for the `i7host` ABI.
//!
//! This is a guest, not a module: build it for `wasm32-unknown-unknown` (or
//! `wasm32v1-none`) and embed the resulting `.wasm` in a native IRIX 7 host
//! module. The host instantiates it, installs the `i7host` imports from
//! `wasm-abi` and calls the exported `run` function.
//!
//! It uses no WASI, no native pointer and no kernel service table. Every
//! pointer passed to `i7host` is a 32-bit offset into this guest's linear
//! memory; the host copies or validates it and never retains it.
//!
//! The current OS slot-B Wasm probe implements an earlier policy draft of
//! `i7host`, so this guest is a compile and import check, not a runnable app
//! yet. See `docs/wasm-apps.md`.

#![no_std]

use wasm_abi::{
    host_console_write, host_now, host_poll, host_submit, host_version, GuestCompletion,
    GuestSlice, HOST_ABI_VERSION,
};

const MESSAGE: &[u8] = b"wasm-hello: i7host ready\n";

/// Entry point the host calls after instantiation. Returns 0 on success.
#[no_mangle]
pub extern "C" fn run() -> i32 {
    // The host refuses a different major version; the guest checks as well so
    // a mismatched pairing is reported instead of silently misbehaving.
    // SAFETY: `host_version` has no arguments and only reads host state.
    if unsafe { host_version() } != HOST_ABI_VERSION {
        return 1;
    }

    // SAFETY: every call passes this guest's linear-memory addresses and
    // lengths; the host validates them against the current memory size. The
    // payload is empty, so the host copies nothing and retains no pointer.
    let now = unsafe { host_now() };
    let token = unsafe { host_submit(1, now, 0, 0, 0, GuestSlice::EMPTY) };
    if token == 0 {
        return 2;
    }

    let mut completion = GuestCompletion {
        event: 0,
        tag: 0,
        status: 0,
        kind: 0,
        flags: 0,
        reserved: 0,
        value0: 0,
        value1: 0,
    };
    // SAFETY: `completion` is a live `GuestCompletion` in linear memory and
    // `max` is 1, matching the host's 32-byte record size.
    let polled = unsafe { host_poll(&mut completion, 1) };
    if polled != 1 || completion.event != token || completion.status < 0 {
        return 3;
    }

    // SAFETY: `MESSAGE` is a static byte slice; the host bounds-checks the
    // length and may block briefly while writing it to the shim console.
    unsafe { host_console_write(MESSAGE.as_ptr(), MESSAGE.len() as u32) };
    unsafe { wasm_abi::host_yield() };
    0
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
