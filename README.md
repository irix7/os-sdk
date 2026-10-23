# IRIX 7 Module SDK

Build loadable modules for [IRIX 7](https://github.com/irix7/os) on the SGI Indy.

The SDK is separate from the operating system. It owns the `module-abi` crate
(ABI version 9) and the OS consumes it at a pinned tag, so there is a single source of
truth for the contract. It also adds the developer-facing pieces:

- `module-sdk`: a `Context` over the kernel service table, plus `entry!` and
  `panic_handler!` macros.
- `targets/` and `linker/`: the n32 target and the flat image layout at `0x8840_0000`.
- `tools/irix7-module.py`: builds, flattens, hashes and prints the load command.
- `template/`: a copyable module crate.
- `examples/hello` and `examples/ticker`: worked examples used by CI.

The kernel loads a module over HTTPS, verifies its SHA-256, and calls its entry point. No
kernel rebuild is needed to add a feature.

## Requirements

- Rust nightly, pinned by `rust-toolchain.toml` (`nightly-2026-10-03`) with `rust-src`,
  `rustfmt` and `clippy`.
- Python 3.
- `llvm-objcopy` from the Rust `llvm-tools` component (the script finds it automatically).

```bash
rustup show          # installs the pinned toolchain if needed
rustup component add rust-src llvm-tools
```

## Quickstart

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

## Template

With `cargo-generate`:

```bash
cargo install cargo-generate
cargo generate --git https://github.com/irix7/os-sdk --subfolder template --name my-module
# the template package is named my-module; rename it if your project differs
python3 tools/irix7-module.py \
  --manifest-path my-module/Cargo.toml \
  --package my-module \
  --linker my-module/linker.ld \
  --output my-module.module
```

Or copy `template/` by hand and rename the package and binary.

The template's `module-sdk` dependency is pinned to the `v0.2.1` tag.

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

For a periodic module, return a vtable with `tick: Some(tick)`. The kernel calls it at most
every 10 ms; return `0` to keep running and non-zero to unregister. The callback receives
the same `*const Services` pointer, so `Context::new` reconstructs the context.

Modules run in **kernel mode**. The SHA-256 digest is the trust boundary; there is no
sandbox. Keep the image under 2 MiB and do not assume relocations: it is linked at
`0x88400000`.

## Repository layout

| Path | Purpose |
|---|---|
| `abi/` | `module-abi`: the versioned service table and vtable |
| `module-sdk/` | `irix7-module-sdk`: `Context`, `entry!`, `panic_handler!` |
| `examples/hello/` | Example module (CI) |
| `template/` | Copyable starting crate |
| `tools/irix7-module.py` | Build, flatten and hash a module |
| `targets/` | `mips3-sgi-irix7-n32` target JSON |
| `linker/` | Flat module linker script |
| `docs/` | Quickstart and ABI notes |

## Licence

GPL-3.0-only. See [`LICENCE`](LICENCE).
