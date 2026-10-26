# wasm-abi

The single source of truth for the WebAssembly core's host-call ABI: version,
guest slice and completion layouts, and the `i7host` import surface.

The OS `guest-abi` crate re-exports this crate as the stable import path. The
ABI is one generic `host_submit` plus completion polling, capability checks,
time, entropy, console and exit.

The current OS slot-B policy probe and `wasm-host` still declare an earlier
policy draft, so a guest built against this crate is not instantiable there yet.
See [`docs/wasm-apps.md`](../docs/wasm-apps.md) for the guest memory rules, the
completion record, the build recipe and that boundary, and
[`examples/wasm-hello`](../examples/wasm-hello) for a guest skeleton.
