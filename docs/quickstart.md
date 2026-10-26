# Quickstart

## Build a module

```bash
git clone https://github.com/irix7/os-sdk
cd os-sdk
python3 tools/irix7-module.py --package hello-sdk
```

The tool runs `cargo build` for the n32 target, flattens the ELF to
`hello-sdk.module`, and prints the size, SHA-256 and a load command.

> SDK main targets module ABI **12**. A kernel that reports an older service table
> rejects a v12 module at entry (`Context::new` fails and the entry returns null).
> Use the SDK tag pinned by your OS release until the ABI 12 loader lands.

### Optional module manifest

Add `--manifest-json manifest.json` to append the version 2 `I7MM` record and
`I7MF` footer. The printed SHA-256 then covers the image, record and footer:

```bash
python3 tools/irix7-module.py \
  --package hello-sdk \
  --manifest-json examples/hello/manifest.json
```

The template ships a starter [`manifest.json`](../template/manifest.json). See
[`module-manifest.md`](module-manifest.md) for the schema, the
`--entry-offset`/`--export interface@version[:offset]` flags, the ABI mapping and
the v1 compatibility rule. Without a manifest option the image is exactly the
flattened payload, as before.

### Module loader ABI

ABI 12 appends `module_arena`, `module_load`, `module_execute`, `module_find`,
`module_find_export`, `module_query` and `module_unload` at `0x54`-`0x6c`. They
let a downloadable manager load verified dependencies into the arena and enter
them with a manager context. The OS kernel does not implement them yet; see
[`module-loader.md`](module-loader.md) for the exact contract and the freeze
exception.

## Build a Wasm guest

A Wasm application is a guest for the `i7host` ABI, not a flat n32 module. Build
the SDK example for a Wasm target:

```bash
cargo build --manifest-path examples/wasm-hello/Cargo.toml --release \
  --target wasm32-unknown-unknown -Z build-std=core
```

The output is `examples/wasm-hello/target/wasm32-unknown-unknown/release/wasm_hello_guest.wasm`.
`-Z build-std=core` builds `core` from `rust-src`, so the wasm32 target component
is not required. The guest is embedded in a native host module; the current OS
slot-B Wasm probe is a policy prototype, not a general app packager. Read
[`wasm-apps.md`](wasm-apps.md) before planning a Wasm application.

## Build a user image

```bash
python3 tools/irix7-user.py --package user-hello-sdk
```

The tool builds the static native n32 user ELF, validates it, and writes
`user-hello.elf` plus a 262144-byte `user-hello.raw` target-2 image.  Attach the
raw image as SCSI target 2; see [`userland.md`](userland.md) for the loader
contract and the boundary between this runtime and POSIX/relibc.

## Load the module on the guest

Serve the directory over the network the Indy can reach:

```bash
python3 -m http.server 18080 --directory .
```

From the guest telnet console (host port `2323` in the iris rig):

```text
module http://10.1.1.3:18080/hello-sdk.module <sha256>
hello from the IRIX 7 module SDK
stage: kernel
...
module: returned 0
```

The kernel's `module` command defaults to HTTPS. An explicit `http://` is accepted for the
local rig; use HTTPS for anything else.

## Read a block

`examples/block` uses the ABI 10 `block_read` service. It reads block 0 of SCSI target 1
and prints the volume-header magic. The example carries its own copy of the flat linker
script:

```bash
python3 tools/irix7-module.py \
  --manifest-path examples/block/Cargo.toml \
  --package block-sdk \
  --linker examples/block/linker.ld \
  --output block-sdk.module
```

The module prints the big-endian magic from the first four bytes; on an SGI volume header
that is:

```text
block: magic 0x4d436f6d
```

`block_read` returns 0 or -1. The module returns status 1 and prints `block: read failed`
when the sector cannot be read; on a service table older than ABI 10 it prints
`block: ABI 10 required` and returns status 1.

## Autoload after a reset

The DS1386 environment at register offset `0x10` holds NUL-separated `name=value`
strings. Set:

```text
mcp.url=http://10.1.1.3:18080/my.module
mcp.sha256=<64 hex digits>
mcp.token=<optional MCP bearer token>
```

After DHCP/DNS the kernel fetches the image, checks the digest and runs it before the
boot menu. A missing or wrong digest prints `autoload: ...` and runs nothing.

## Use the module template

```bash
cargo install cargo-generate
cargo generate --git https://github.com/irix7/os-sdk --subfolder template --name my-module
# rename the template package if your project differs
python3 tools/irix7-module.py \
  --manifest-path my-module/Cargo.toml \
  --package my-module \
  --linker my-module/linker.ld \
  --manifest-json my-module/manifest.json \
  --output my-module.module
```

The generated `manifest.json` uses format 2 and names `my-module` at version
`0.3.0` and ABI 1.10, matching the template's pinned `module-sdk` tag. Rename the
package and update `name`, `version` and `abi_minor` together when you change
either; drop `--manifest-json` to build a standalone image with no manifest.

## Use the user template

```bash
cargo generate --git https://github.com/irix7/os-sdk --subfolder templates/user --name my-user-program
python3 tools/irix7-user.py \
  --manifest-path my-user-program/Cargo.toml \
  --package my-user-program \
  --linker my-user-program/linker.ld \
  --output my-user-program/my-user-program.elf
```

## Where to go next

- [`../module-sdk/src/lib.rs`](../module-sdk/src/lib.rs): the module `Context` API.
- [`../wasm-abi/src/lib.rs`](../wasm-abi/src/lib.rs): the `i7host` Wasm import surface.
- [`../user-sdk/src/lib.rs`](../user-sdk/src/lib.rs): the user runtime API.
- [`abi.md`](abi.md): the module service-table version and append-only rule.
- [`module-manifest.md`](module-manifest.md): the manifest JSON, `I7MM`/`I7MF`
  v1/v2 format, dependency digests, entry/export offsets and ABI mapping.
- [`module-loader.md`](module-loader.md): the ABI 12 arena/load/execute/lookup
  contract, statuses, freeze exception and remaining OS work.
- [`wasm-apps.md`](wasm-apps.md): what a Wasm app is, its build and embedding
  path, and the current runtime boundary.
- [`user-abi.md`](user-abi.md): the provisional user syscall table and layouts.
- [`userland.md`](userland.md): the userland boundary, loader contract and
  target-2 packing.
- [IRIX 7 module specification](https://github.com/irix7/os/blob/main/docs/spec/modules.md):
  kernel-side lifecycle, cache maintenance and the boot autoload.
