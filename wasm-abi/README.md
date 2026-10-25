# wasm-abi

The single source of truth for the WebAssembly core's host-call ABI: version,
guest slice and completion layouts, and the `i7host` import surface.

The OS `guest-abi` crate re-exports this crate so the core, the shim and the
SDK cannot drift. The ABI is one generic `host_submit` plus completion
polling, capability checks, time, entropy, console and exit.
