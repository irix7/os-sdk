# Quickstart

## Build a module

```bash
git clone https://github.com/irix7/os-sdk
cd os-sdk
python3 tools/irix7-module.py --package hello-sdk
```

The tool runs `cargo build` for the n32 target, flattens the ELF to
`hello-sdk.module`, and prints the size, SHA-256 and a load command.

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
  --output my-module.module
```

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
- [`../user-sdk/src/lib.rs`](../user-sdk/src/lib.rs): the user runtime API.
- [`abi.md`](abi.md): the module service-table version and append-only rule.
- [`user-abi.md`](user-abi.md): the provisional user syscall table and layouts.
- [`userland.md`](userland.md): the userland boundary, loader contract and
  target-2 packing.
- [IRIX 7 module specification](https://github.com/irix7/os/blob/main/docs/spec/modules.md):
  kernel-side lifecycle, cache maintenance and the boot autoload.
