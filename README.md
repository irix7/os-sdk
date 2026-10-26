# IRIX 7 SDK

Build software for [IRIX 7](https://github.com/irix7/os) on the SGI Indy.  The SDK has
two distinct targets:

| Target | Output | Load address | Runs as |
|---|---|---|---|
| **Kernel module** | flat binary, SHA-256 checked | `0x8840_0000` | kernel mode in the kernel address space |
| **User program** | static n32 ELF + 256 KiB raw SCSI image | `0x1000_0000` | user mode, loaded from SCSI target 2 |

They are not interchangeable.  A module is not a process, and a user program cannot
call the module service table.

The SDK is separate from the operating system.  It owns the `module-abi` crate
(ABI version 10), the userland ABI and target, the linkers, the build tools and the
templates.  The OS consumes the module ABI at a pinned tag so there is a single
source of truth for the contract.

## What is in the SDK

- `module-sdk`: a `Context` over the kernel service table, plus `entry!` and
  `panic_handler!` macros.
- `manifest`: the build-time module manifest model: the OS `I7MM` record and `I7MF`
  footer, with the same encoder, decoder and limits as the OS resolver.
- `wasm-abi`: the draft `i7host` host-call ABI and guest layouts for Wasm applications.
- `user-abi`: the provisional native n32 userland syscall numbers, flags and C layouts.
- `user-sdk`: the static user runtime: `_start`, arguments/environment, console,
  descriptors, directories and clocks.
- `targets/` and `linker/`: the kernel/module target and flat linker, plus the
  distinct native user target and user linker.
- `tools/irix7-module.py`: builds, flattens, hashes and prints the module load command,
  optionally appending a manifest and footer.
- `tools/irix7-user.py`: builds a user image, validates the n32 ELF and packs the
  256 KiB target-2 image.
- `template/` and `templates/user/`: copyable module and user crates.
- `examples/hello`, `examples/ticker`, `examples/block`: module examples used by CI.
- `examples/user-hello`, `examples/user-probe`: static user examples used by CI.

## Requirements

- Rust nightly, pinned by `rust-toolchain.toml` (`nightly-2026-10-03`) with `rust-src`,
  `rustfmt` and `clippy`.
- Python 3.
- For modules, `llvm-objcopy` from the Rust `llvm-tools` component (the script finds it
  automatically).

```bash
rustup show          # installs the pinned toolchain if needed
rustup component add rust-src llvm-tools
```

## Module quickstart

```bash
git clone https://github.com/irix7/os-sdk
cd os-sdk
python3 tools/irix7-module.py --package hello-sdk
```

Add `--json` for machine-readable output. The last lines are the flat image, its size,
its SHA-256, and a copy-paste load command:

```text
module: /home/you/os-sdk/hello-sdk.module
size:   1234 bytes
sha256: <64 hex digits>
load:   module http://<host>:<port>/hello-sdk.module <64 hex digits>
```

Serve the image and load it from the guest telnet console:

```bash
python3 -m http.server 18080 --directory .
```

```text
module http://10.1.1.3:18080/hello-sdk.module <sha256>
```

Without a scheme the kernel uses HTTPS; explicit `http://` is for the local iris rig.

For a long-running module, put the URL and digest in the guest's NVRAM environment and it
will autoload after a reset:

```text
mcp.url=http://10.1.1.3:18080/my.module
mcp.sha256=<64 hex digits>
mcp.token=<optional bearer token for MCP>
```

The environment lives at DS1386 register offset `0x10`; see the OS
[module specification](https://github.com/irix7/os/blob/main/docs/spec/modules.md).

## Module manifests and runtime dependencies

A module may declare a manifest: its exact name and version, the service-table ABI it
was built for, exact dependencies with SHA-256 digests, and the interfaces it exports.
Pass `--manifest-json` (or the explicit `--name`/`--version`/`--abi-*`/`--depends`/
`--export` flags) and the tool appends the version 1 `I7MM` record and `I7MF` footer
after the flattened image:

```json
{
  "format": 1,
  "name": "mesh.transport",
  "version": "1.2.3",
  "abi_major": 1,
  "abi_minor": 11,
  "depends": [
    {
      "name": "mesh.codec",
      "version": "1.0.0",
      "sha256": "<64 hex digits over the dependency image>"
    }
  ],
  "exports": [{ "interface": "irix7.mesh.transport", "version": 1 }]
}
```

```bash
python3 tools/irix7-module.py --package my-module --manifest-json my-module/manifest.json
```

The printed SHA-256 then covers image, record and footer, and is the digest the
`module` command and the `mcp.sha256` autoload key check. A module built without a
manifest option is byte-for-byte the same flat image as before and remains a valid
standalone module. Manifests change no service ABI: the kernel does not read the
trailing data, and the entry point stays at offset 0. See
[`docs/module-manifest.md`](docs/module-manifest.md) for the JSON schema, the exact
wire format, the ABI mapping and the current limits. The OS resolver in
`crates/module-resolver` remains the runtime/host authority.

## Userland quickstart

```bash
python3 tools/irix7-user.py --package user-hello-sdk
```

The tool builds with the SDK's native n32 target and user linker, then validates the
result before writing it:

```text
elf:    /home/you/os-sdk/user-hello.elf
image:  /home/you/os-sdk/user-hello.raw
size:   12345 bytes (ELF)
image:  262144 bytes (raw SCSI target 2)
entry:  0x10000000
sha256: <64 hex digits>
```

Attach `user-hello.raw` as SCSI target 2.  The kernel reads LBA 0 and enters the image
after boot; the telnet `userelf` command reloads the same image.  `--json` prints
`elf`, `image`, `elf_size`, `image_size`, `sha256`, `entry` and the allocated section
names.

This is a **static native bring-up runtime**, not POSIX.  There is no process table,
`fork`/`exec`, signals, TLS, `termios`, sockets or general filesystem.  The production
POSIX C layer is the separate [relibc fork](https://github.com/irix7/relibc); it is not
part of this SDK.  See [`docs/userland.md`](docs/userland.md) for the boundary and
[`docs/user-abi.md`](docs/user-abi.md) for the provisional syscall table.

## Templates

Modules, with `cargo-generate`:

```bash
cargo install cargo-generate
cargo generate --git https://github.com/irix7/os-sdk --subfolder template --name my-module
# the template package is named my-module; rename it if your project differs
python3 tools/irix7-module.py \
  --manifest-path my-module/Cargo.toml \
  --package my-module \
  --linker my-module/linker.ld \
  --manifest-json my-module/manifest.json \
  --output my-module.module
```

The generated `manifest.json` is an optional starter: it names `my-module` at version
`0.3.0` with ABI 1.10, matching the template's pinned `module-sdk` tag. Drop the flag
for a standalone image, or update the manifest when you rename or bump the crate.

User programs:

```bash
cargo generate --git https://github.com/irix7/os-sdk --subfolder templates/user --name my-user-program
python3 tools/irix7-user.py \
  --manifest-path my-user-program/Cargo.toml \
  --package my-user-program \
  --linker my-user-program/linker.ld \
  --output my-user-program/my-user-program.elf
```

Or copy the template directory by hand and rename the package and binary.  The module
template's `module-sdk` dependency is pinned to `v0.3.0`; the user template is pinned to
`v0.4.0`, the first release with `irix7-user-sdk`.

## Writing a module

```rust
#![no_std]
#![no_main]

use module_sdk::{Context, ModuleVtable};

static VTABLE: ModuleVtable = ModuleVtable { status: 0, tick: None };

fn run(context: &Context) -> *const ModuleVtable {
    context.println("hello from my module");
    &VTABLE
}

module_sdk::entry!(run);
module_sdk::panic_handler!();
```

`Context` exposes the console, monotonic clock, shared status/net/mem/time blocks, the
BGR screen shadow, allocation, two TCP slots, PROM environment lookup and reset/poweroff.
See [`module-sdk/src/lib.rs`](module-sdk/src/lib.rs).

ABI 10 adds `Context::block_read(target, lba, &mut sector)` for one 512-byte SCSI sector.
ABI 11 adds `Context::block_write(target, lba, &sector)`; the kernel validates the buffer
pointer and the device controls durability.
[`examples/block`](examples/block) reads block 0 of target 1 and prints the volume-header
magic; it is the reference for the block service:

```rust
let mut sector = [0u8; 512];
if context.block_read(1, 0, &mut sector) {
    let magic = u32::from_be_bytes([sector[0], sector[1], sector[2], sector[3]]);
    // print "block: magic 0x...."
}
```

For a periodic module, return a vtable with `tick: Some(tick)`. The kernel calls it at most
every 10 ms; return `0` to keep running and non-zero to unregister. The callback receives
the same `*const Services` pointer, so `Context::new` reconstructs the context.

Modules run in **kernel mode**. The SHA-256 digest is the trust boundary; there is no
sandbox. Keep the image under 2 MiB and do not assume relocations: it is linked at
`0x88400000`.

## Writing a WebAssembly app

A Wasm app is not a flat n32 module. It is a guest compiled for
`wasm32-unknown-unknown` (or `wasm32v1-none`) that imports the `i7host` interface from
the `wasm-abi` crate. It gets no WASI, no kernel service table and no native pointer:
every pointer is a 32-bit offset into the guest's linear memory.

```rust
#![no_std]

use wasm_abi::{host_console_write, host_version, HOST_ABI_VERSION};

#[no_mangle]
pub extern "C" fn run() -> i32 {
    if unsafe { host_version() } != HOST_ABI_VERSION {
        return 1;
    }
    let message = b"hello from Wasm\n";
    unsafe { host_console_write(message.as_ptr(), message.len() as u32) };
    0
}
// A guest also needs a #[panic_handler]; see examples/wasm-hello.
```

[`examples/wasm-hello`](examples/wasm-hello) is a complete guest skeleton using
`host_submit`, `host_poll` and the completion record:

```bash
cargo build --manifest-path examples/wasm-hello/Cargo.toml --release \
  --target wasm32-unknown-unknown -Z build-std=core
```

The `.wasm` is embedded in a native host module (`include_bytes!`) that provides the
imports; there is no SDK packager and no general Wasm app loader yet. The OS slot-B
`modules/wasm` image is a policy/interpreter prototype whose current guest uses an
earlier `i7host` draft, so a guest built against `wasm-abi` is not yet runnable there.
[`docs/wasm-apps.md`](docs/wasm-apps.md) covers the full import surface, pointer rules,
the completion ring and the current boundary honestly.

## Writing a user program

```rust
#![no_std]
#![no_main]

use user_sdk::{entry, panic_handler, println, Context};

fn run(context: &Context) -> i32 {
    println!("hello from my user program");
    println!("argc={}", context.argc());
    0
}

entry!(run);
panic_handler!();
```

`Context` exposes `args()`, `env_vars()`, `env(name)`, `write`, `println` and `exit`.
`user_sdk::fs` adds `File`, `Dir`, `stat`, `getcwd` and `chdir`; `user_sdk::time` adds
`clock`, `monotonic`, `realtime` and `sleep`.  The return value of `run` is the exit
status.  The runtime is `no_std`; there is no allocator, so programs use fixed buffers.

The user target remains distinct from the kernel/module target.  The user ELF is
`ELF32`, big-endian, n32 (`e_flags` ABI2), MIPS III, static and linked at
`0x1000_0000`.  `tools/irix7-user.py` rejects a wrong class, endianness, ABI, machine,
PIC flag, dynamic segment, interpreter, TLS segment, undefined symbol, entry outside
`.text`, section outside `PT_LOAD` or an ELF larger than 256 KiB.

## Repository layout

| Path | Purpose |
|---|---|
| `abi/` | `module-abi`: the versioned module service table and vtable |
| `module-sdk/` | `irix7-module-sdk`: module `Context`, `entry!`, `panic_handler!` |
| `user-abi/` | `irix7-user-abi`: provisional user syscalls, flags and layouts |
| `user-sdk/` | `irix7-user-sdk`: static user `_start`, context, console, fs, clocks |
| `examples/hello/` | Example module (CI) |
| `examples/ticker/` | Periodic callback example (CI) |
| `examples/block/` | Block-service example with its own linker script (CI) |
| `examples/wasm-hello/` | Wasm `i7host` guest skeleton (CI) |
| `examples/user-hello/` | Console/args/environment/BSS user example (CI) |
| `examples/user-probe/` | Filesystem and clock user probe (CI) |
| `manifest/` | `irix7-module-manifest`: `I7MM` record and `I7MF` footer model |
| `template/` | Copyable module crate with an optional `manifest.json` |
| `templates/user/` | Copyable user crate |
| `testdata/` | Cross-implementation manifest vectors from the OS encoder |
| `tools/irix7-module.py` | Build, flatten, optionally manifest, and hash a module |
| `tools/irix7_manifest.py` | `I7MM`/`I7MF` encoder, decoder and validation |
| `tools/irix7-user.py` | Build, validate and pack a user image |
| `tools/check-os-sync.py` | Compare the SDK target/ABI with an OS checkout |
| `targets/` | `mips3-sgi-irix7-n32` and `mips3-sgi-irix7-native-n32` JSON |
| `linker/` | Flat module linker and static user linker |
| `docs/` | Module ABI, manifests, Wasm apps, userland and quickstarts |

## Licence

GPL-3.0-only. See [`LICENCE`](LICENCE).
