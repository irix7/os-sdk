# wasm-hello

Minimal IRIX 7 Wasm guest for the `i7host` ABI in [`wasm-abi`](../../wasm-abi).

It is a wasm32 guest, not a flat n32 module: it is not linked at `0x8840_0000`, cannot
be loaded with the kernel `module` command, and never receives the kernel service table
or a native pointer. A native host module embeds the `.wasm` and provides the imports.

Build:

```bash
cargo build --release --target wasm32-unknown-unknown -Z build-std=core
```

Output:

```text
target/wasm32-unknown-unknown/release/wasm_hello_guest.wasm
```

The guest imports only `i7host` (`host_version`, `host_now`, `host_submit`, `host_poll`,
`host_console_write`, `host_yield`), exports `run` and `memory`, and uses no WASI. It
checks `HOST_ABI_VERSION`, submits one operation, polls one completion and writes a
console line.

## Current limitation

The OS slot-B `modules/wasm` probe implements an earlier policy draft of `i7host`
(five-argument `host_submit`, policy hooks, no `host_cap_derive`, `host_random`,
`host_console_read` or `host_exit`). A guest built against `wasm-abi` cannot be
instantiated by that host yet. This example is therefore a compile and import check and
a starting point for a v1 host, not a runnable probe.

See [`docs/wasm-apps.md`](../../docs/wasm-apps.md) for the full surface, pointer rules,
completion record and the runtime boundary. `tools/tests/test_wasm_guest.py` checks the
built module's imports and exports.
