# Quickstart

## Build the example

```bash
git clone https://github.com/irix7/os-sdk
cd os-sdk
python3 tools/irix7-module.py --package hello-sdk
```

The tool runs `cargo build` for the n32 target, flattens the ELF to
`hello-sdk.module`, and prints the size, SHA-256 and a load command.

## Load it on the guest

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

## Use the template

```bash
cargo install cargo-generate
cargo generate --git https://github.com/irix7/os-sdk --subfolder template --name my-module
python3 tools/irix7-module.py \
  --manifest-path my-module/Cargo.toml \
  --package my-module \
  --linker my-module/linker.ld \
  --output my-module.module
```

## Where to go next

- [`../module-sdk/src/lib.rs`](../module-sdk/src/lib.rs): the full `Context` API.
- [`abi.md`](abi.md): the service-table version and append-only rule.
- [IRIX 7 module specification](https://github.com/irix7/os/blob/main/docs/spec/modules.md):
  kernel-side lifecycle, cache maintenance and the boot autoload.
