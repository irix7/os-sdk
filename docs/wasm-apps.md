# WebAssembly applications

**Status:** draft guest ABI (`wasm-abi` v1) and a compile-checked guest example. There is
no general Wasm app loader, packager or SDK runtime yet. This page states what exists,
what is only a prototype, and where the boundary is.

A Wasm application on IRIX 7 is a guest compiled for a 32-bit WebAssembly target that
imports the `i7host` interface from [`wasm-abi`](../wasm-abi). It is **not** a flat n32
module: it is not linked at `0x8840_0000`, it cannot be loaded with the kernel's `module`
command, and it never receives the kernel service table, a native MIPS pointer or WASI.

The guest is data inside a native **host module** — an ordinary flat n32 module that
embeds the `.wasm`, runs a WebAssembly interpreter, installs `i7host` and calls the
guest's exported `run`. Today that host is the OS slot-B `modules/wasm` prototype; see
[Runtime today](#runtime-today).

## The `i7host` import surface

`wasm-abi` declares `HOST_ABI_VERSION = 1` and a single import module, `i7host`, with
`#[link(wasm_import_module = "i7host")]`:

| Import | Signature | Purpose |
|---|---|---|
| `host_version` | `() -> u32` | Host ABI version. The guest must refuse a different major. |
| `host_submit` | `(op: u32, a0: u64, a1: u64, a2: u64, a3: u64, payload: GuestSlice) -> u64` | Queue an operation. Returns a non-zero token, or zero if it could not be queued. |
| `host_poll` | `(out: *mut GuestCompletion, max: u32) -> u32` | Drain up to `max` completions into guest memory; returns the number written. |
| `host_cancel` | `(token: u64) -> i32` | Best-effort cancellation. Zero if found, negative otherwise. The operation may already have committed. |
| `host_cap_check` | `(handle: u64, rights: u32, object: *const u8) -> i32` | Check a capability against a 32-byte object ID. Zero on success. |
| `host_cap_derive` | `(parent: u64, rights: u32, not_after: u64, out: *mut u8, out_len: u32) -> i32` | Write a 320-byte attenuated token; returns its length or a negative status. |
| `host_now` | `() -> u64` | Monotonic nanoseconds since boot. |
| `host_random` | `(out: *mut u8, len: u32) -> i32` | Fill guest memory with entropy. Zero on success. |
| `host_console_write` | `(ptr: *const u8, len: u32)` | Write to the shim console. Bounded; may block briefly. |
| `host_console_read` | `(ptr: *mut u8, len: u32) -> u32` | Read console input without blocking. Returns bytes read. |
| `host_yield` | `()` | Cooperative scheduling hint, not a trap. |
| `host_exit` | `(status: i32) -> !` | Terminate the current actor. Does not return. |

The operation code passed to `host_submit` is the OS `sys-primitives::OpCode`
discriminant (`Nop = 0`, `Hello = 1`, `Auth = 2`, `Lookup = 0x10`, `Read = 0x12`,
`Write = 0x13`, `Invoke = 0x20`, and so on). The SDK declares the transport, not the
operation set; a guest should take the numbers from the OS crate or its own frozen
constants rather than reusing arbitrary values.

The `a0..a3` arguments are operation-specific handles, offsets, lengths or flags. Their
meaning is defined by the operation, not by `wasm-abi`. `payload` is an optional inline
byte range in guest memory; the host must copy it before returning.

### Wasm lowering

Rust's wasm C ABI passes a two-word aggregate such as `GuestSlice` indirectly. The guest
example's `host_submit` import therefore has the wasm type
`(i32, i64, i64, i64, i64, i32) -> i64`: the last `i32` is a pointer to an 8-byte
`{ ptr: u32, len: u32 }` in guest memory, not the two offsets inline. A host must
validate that struct pointer, read the two words, then validate `ptr + len` for the
payload. Pointers (`*const u8`, `*mut GuestCompletion`, …) are single `i32` offsets.

Wasm imports are matched by module, name and exact function type at instantiation. A
host with a different arity (for example a five-argument `host_submit`) rejects the
guest rather than silently ignoring arguments.

## Guest memory and pointer rules

Every pointer in the ABI is a 32-bit offset into the guest's linear memory. At the wasm
boundary the Rust pointer types in `wasm-abi` lower to `i32`; a guest must never pass a
value that is not an offset into its own memory.

- `GuestSlice { ptr: u32, len: u32 }` describes a byte range. `GuestSlice::EMPTY` is
  `{ 0, 0 }`.
- A range must satisfy `ptr + len <= memory.size()` without 32-bit overflow. The host
  validates at call time; it does not remember a range after the call.
- Input buffers are copied or read before the call returns. The host must never retain a
  guest pointer, and the guest must not assume the host keeps the bytes alive.
- Output buffers must be large enough for the documented fixed sizes: 32 bytes for an
  object ID, 320 bytes for a derived capability token, `max * 32` bytes for `host_poll`.
- A `memory.grow` between calls is fine; a stale offset is only meaningful against the
  memory that exists at the next call.
- Wasm is little-endian. IRIX 7 runs on big-endian MIPS, so the host encodes multi-byte
  fields explicitly rather than blanket-swapping a buffer. Byte strings and object IDs
  are passed unchanged.

`wasm-host` is the reference host validation. `read_memory`/`write_memory` reject a
negative `i32` pointer, require an exported `memory`, and use the wasmi interpreter's
bounds-checked `Memory::read`/`Memory::write`. A failed read returns `-1` (or `0` for
`host_poll`); it never reads past the current memory. The current policy host also caps
paths at 160 bytes and blobs at 512 bytes and validates path bytes as UTF-8 before
calling a native hook. Those are probe limits, not part of `wasm-abi`.

## Completions

`host_submit` returns a token and the result arrives later through `host_poll`. A
completion is the 32-byte little-endian record declared as `GuestCompletion` and mirrored
by `sys_primitives::Completion`:

```text
0   event    u64   submission token
8   tag      u32   request tag
12  status   i32   negative is an error
16  kind     u8
17  flags    u8
18  reserved u16   zero
20  value0   u32   kind-specific
24  value1   u64   kind-specific
```

The guest owns the output buffer and passes it to `host_poll`; the host writes records
there and returns how many. This is not a shared-memory ring: the host keeps its own
queue and drains it into guest memory. A zero token means the submission was not queued;
`host_cancel` is best-effort and a completion may already be waiting.

## Runtime today

The only in-tree host is the OS slot-B `modules/wasm` probe:

- It is a flat n32 module linked at `0x8848_0000` for slot B, limited to 1,572,864 bytes.
  It is loaded with `module b …` or the NVRAM `wasm.url`/`wasm.sha256` pair, so slot A
  (MCP) stays live.
- It embeds one guest `.wasm` at build time (`include_bytes!`), instantiates it with the
  `no_std` `wasmi` interpreter and 10,000,000 fuel, and calls its `run` export.
- The guest runs the M1 local policy: it stages `/etc`, `/etc/motd`, the root home, an
  actor and a capability, validates the allow and deny cases, and commits atomically.
- The host supplies the native policy table through `wasm-host::HostHooks` /
  `wasm-host::PolicyHooks`; the kernel's native `m1:` policy remains the fallback.

**This probe is not a general Wasm app packager.** There is no upload, install, slot
allocation, per-app isolation, WASI or module multiplexer. One guest is compiled into one
host image.

The probe's guest also uses an **earlier policy draft** of `i7host` declared in its own
`extern "C"` block, not `wasm-abi`:

| SDK `wasm-abi` v1 | Slot-B policy probe |
|---|---|
| `host_submit(op, a0..a3, payload)` (six arguments) | `host_submit(op, a0..a3)` (five arguments, no payload) |
| `host_cap_derive`, `host_random`, `host_console_read`, `host_exit` | absent |
| no policy-specific imports | `host_node_id`, `host_policy_*`, `host_ns_*`, `host_actor_*`, `host_cap_issue`, `host_cap_rights`, `host_policy_stats` |

Wasm imports are matched by module, name and function type at instantiation, so a guest
built against `wasm-abi` cannot be instantiated by the current slot-B host. Closing that
gap is an OS host change (install the v1 import set or change the ABI); it is out of
scope for the SDK and does not change the module service ABI, kernel or ROM.

The [example](#build-a-guest) is therefore a compile and import check, not a runnable
probe. Do not describe a guest built here as loadable on iris today.

## Build a guest

The SDK example is [`examples/wasm-hello`](../examples/wasm-hello):

```bash
cargo build --manifest-path examples/wasm-hello/Cargo.toml --release \
  --target wasm32-unknown-unknown -Z build-std=core
```

Output:

```text
examples/wasm-hello/target/wasm32-unknown-unknown/release/wasm_hello_guest.wasm
```

`-Z build-std=core` builds `core` from `rust-src`, so only the pinned nightly and the
`rust-src` component are required; the wasm32 target component is not. `wasm32v1-none`
is also a valid guest target for the crate.

The guest is a `cdylib` with `#![no_std]`, a `#[panic_handler]` and one exported entry:

```rust
#[no_mangle]
pub extern "C" fn run() -> i32 {
    if unsafe { wasm_abi::host_version() } != wasm_abi::HOST_ABI_VERSION {
        return 1;
    }
    // host_submit -> token -> host_poll -> GuestCompletion
    0
}
```

The example checks the ABI, submits one operation, polls one completion and writes a
console line. It adds no runtime or ABI semantics; it only uses the declarations in
`wasm-abi`.

## Embedding and packaging

A host module is an ordinary n32 module:

1. Build the guest `.wasm` as above.
2. Embed it with `include_bytes!` in the host crate, or pass it to the host at build
   time.
3. Instantiate it with an interpreter and the full `i7host` import set.
4. Call the exported `run`; keep it within fuel and trap policy.

The OS probe shows the shape but implements the policy draft, so it is a reference for
embedding, not a v1 host. There is no `irix7-module.py --wasm` and no `.i7wasm` container
format. A Wasm guest is not a module image and cannot be hashed and loaded as one on its
own; the digest that matters is the digest of the host module image that embeds it.

A host module may carry a module manifest (`I7MM`/`I7MF`) like any other flat image; see
[`module-manifest.md`](module-manifest.md). Dependency declarations belong to the host
module, not to the guest. The guest has no name, version or ABI record of its own beyond
`HOST_ABI_VERSION`.

## What is not available

- WASI, libc, filesystem or socket calls from the guest.
- A native pointer, kernel `Services` table or ambient capability in the guest.
- Multiple guests per host image, guest upload, per-app slots or an app store.
- A stable operation set: `OpCode` values are OS-owned and may be extended.
- Signatures, attestation or a guest digest separate from the host image.
- Determinism or timing guarantees on MIPS beyond the interpreter's configured fuel
  metering; those are not yet proven.

## Tests

```bash
cargo build --manifest-path examples/wasm-hello/Cargo.toml --release \
  --target wasm32-unknown-unknown -Z build-std=core
IRIX7_WASM_GUEST=examples/wasm-hello/target/wasm32-unknown-unknown/release/wasm_hello_guest.wasm \
  python3 tools/tests/test_wasm_guest.py
```

The test parses the WebAssembly import and export sections and checks that the guest
imports only `i7host` (no WASI), declares the core v1 host calls with their exact wasm
types, exports `run` and `memory`, and is a WebAssembly version 1 module. There are no
runtime tests because there is no v1 host.

## References

- [`wasm-abi/src/lib.rs`](../wasm-abi/src/lib.rs): the import declarations,
  `GuestSlice`, `GuestCompletion` and `HOST_ABI_VERSION`.
- OS [`docs/spec/modules.md`](https://github.com/irix7/os/blob/main/docs/spec/modules.md):
  slot layout and the slot-B Wasm policy module.
- OS [`docs/spec/system.md`](https://github.com/irix7/os/blob/main/docs/spec/system.md)
  §3.2: the Wasm core and shim model.
- OS [`docs/plan/milestones.md`](https://github.com/irix7/os/blob/main/docs/plan/milestones.md):
  the M1 Wasm policy status and limits.
- OS [`modules/wasm`](https://github.com/irix7/os/tree/main/modules/wasm): the embedding
  probe and its current guest.
- OS [`crates/wasm-host`](https://github.com/irix7/os/tree/main/crates/wasm-host):
  interpreter host, memory validation, completions and hooks.
