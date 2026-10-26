# Module manifests and runtime dependencies

A module image may carry a **version 2 manifest** after its flattened payload. The
manifest names the module, states the exact version and service-table ABI it was built
for, declares exact runtime dependencies with their SHA-256 digests, and lists the
interfaces it exports with the image-relative offset of each C function table. It also
names the manager entry point.

This document is the SDK's build-time surface for the format. The OS
[`module-resolver`](https://github.com/irix7/os/tree/5a7696e/crates/module-resolver)
implementation is the host-side resolver and the authority for version 1 bytes;
[`docs/spec/modules.md` sections 8.1 and 8.2](https://github.com/irix7/os/blob/5a7696e/docs/spec/modules.md)
are the version 1 contract. The v1 vectors in [`testdata/`](../testdata) are canonical OS
output that both SDK decoders must accept.

Version 2 is the SDK's loader-binding extension: it adds `entry_offset` and per-export
table offsets. The kernel does **not** read or validate a manifest; the downloadable
manager parses it, passes the identity and offsets to `module_load`, and drives the ABI 12
loader ([`module-loader.md`](module-loader.md)). The kernel records what the manager
passes; it does not cross-check the record.
A manifest is trailing data after the image, so the entry at offset 0 and the service
table are unaffected. A module image without a manifest remains a valid standalone
module; the build tool preserves that behaviour when no manifest option is given.

## Image layout

```text
image bytes | I7MM manifest record | I7MF footer
```

The footer locates the record without moving anything in the image:

```text
I7MF | u16 footer version=1 | u16 reserved=0
     | u32 manifest offset | u32 manifest length
```

The offset and length must describe a range that ends exactly at the footer. The
SHA-256 of the **complete** file — image, record and footer — is the integrity boundary
and the digest used by the `module` command, the NVRAM autoload keys and the ABI 12
`module_load` request. The footer has no checksum of its own.

The version 2 record is little-endian:

```text
I7MM | u16 format=2 | u16 abi major | u16 abi minor
     | u32 entry offset
     | u16 name length | UTF-8 name
     | u16 version length | UTF-8 version
     | u16 dependency count
     | (u16 name length, name, u16 version length, version, 32-byte SHA-256)*
     | u16 export count
     | (u16 interface length, interface, u16 export version, u32 table offset)*
```

Version 1 is the same record without the entry offset and the per-export offsets:

```text
I7MM | u16 format=1 | u16 abi major | u16 abi minor
     | u16 name length | name | u16 version length | version
     | u16 dependency count | (...)*
     | u16 export count | (u16 interface length, interface, u16 export version)*
```

The decoder accepts both; a version 1 record decodes with `entry_offset == 0` and every
export offset zero. The encoder **only emits version 2**. Version 2 is not a
byte-compatible extension of version 1: it inserts `entry_offset` before the name and a
`u32` in each export record, so the version field selects the decoder and v1 bytes are
never reinterpreted. A v1-only reader (including the OS `module-resolver` at 5a7696e)
rejects a v2 record with `UnsupportedVersion(2)`; see
[OS changes still required](#os-changes-still-required).

Records are limited to 64 KiB. There is no CRC: the image digest covers the record.
Decoders reject bad magic, an unsupported format or footer version, a non-zero reserved
field, truncation, trailing bytes, invalid UTF-8, an out-of-range footer and an oversized
record. A present but malformed footer is an error, never an older standalone image.

### Offsets

- `entry_offset` is the image-relative address the manager passes to
  `module_execute`. `0` is the legacy `_start(const Services *)` entry. A non-zero
  offset must be 4-byte aligned and must leave at least four bytes inside the flattened
  payload. The kernel performs the data-cache writeback and instruction-cache invalidate
  before entering.
- `exports[].offset` is the image-relative address of the interface's C function table.
  `0` means "interface declaration only, no table". A non-zero offset is 4-byte aligned
  and must fit inside the flattened payload. The manager copies it into the `ModuleExport`
  list passed to `module_load`; the kernel returns `region_base + offset` through
  `module_find_export`. No symbol relocation or ambient pointer is involved.

Both are checked against the built payload by `tools/irix7-module.py` before the record
is wrapped. The manifest crate is a codec and does not know the image size.

## Authoring a manifest in JSON

Create `manifest.json` beside the crate:

```json
{
  "format": 2,
  "name": "mesh.transport",
  "version": "1.2.3",
  "abi_major": 1,
  "abi_minor": 12,
  "entry_offset": 0,
  "depends": [
    {
      "name": "mesh.codec",
      "version": "1.0.0",
      "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
    }
  ],
  "exports": [
    { "interface": "irix7.mesh.transport", "version": 1, "offset": 128 }
  ]
}
```

| Field | Required | Meaning |
|---|---|---|
| `format` | no | Manifest format. Only `2` is accepted; the default is `2`. `1` is decode-only. |
| `name` | yes | Stable module name, `A-Z a-z 0-9 . _ + -`. |
| `version` | yes | Exact version, same character set as `name`. Ranges are not supported. |
| `abi_major` | yes | Service-table generation. The only generation this SDK builds is `1`. |
| `abi_minor` | yes | Service-table revision, at most the SDK's `ABI_VERSION` (12). |
| `entry_offset` | no | Image-relative manager entry; default `0` (legacy `_start`). |
| `depends` | no | Exact dependencies; each needs `name`, `version` and `sha256`. |
| `exports` | no | Exports; each needs `interface`, a `u16` `version` and optional `offset`. |

Unknown keys are rejected so a typo such as `dependencies` cannot be ignored.
`depends` and `exports` default to empty lists.

`sha256` is 64 hexadecimal digits over the dependency's whole image, including its own
manifest and footer. Use the digest printed when that dependency was built. The release
side of [issue #217](https://github.com/irix7/os/issues/217) will publish the same value;
the package index has no `depends` field yet.

The SDK applies these build-time rules on top of the wire format:

- names, versions and interfaces must be non-empty and use `A-Z a-z 0-9 . _ + -`;
- a dependency may appear once; two versions of the same dependency name are a
  conflict in the v1 exact-version model and are rejected;
- an export may not repeat the same `interface` and `version` pair (two versions of one
  interface are allowed);
- `entry_offset` and every non-zero export offset must be 4-byte aligned, and must fit
  inside the flattened image;
- `abi_major` must be `1` and `abi_minor` must not be newer than the service table in
  [`abi/src/lib.rs`](../abi/src/lib.rs);
- the encoded record must fit the 64 KiB bound and every length field must fit a `u16`.

## Building with a manifest

```bash
python3 tools/irix7-module.py \
  --package my-module \
  --manifest-json my-module/manifest.json
```

The tool builds and flattens the image as before, appends the record and footer, then
prints the final size and digest:

```text
module: /home/you/os-sdk/my-module.module
size:   4586 bytes
sha256: <64 hex digits over image + manifest + footer>
image:  4450 bytes (flattened image before the manifest)
manifest: mesh.transport@1.2.3 abi 1.12, entry 0, 1 dependency, 1 export (record 120 bytes, footer 16 bytes)
load:   module http://<host>:<port>/my-module.module <64 hex digits>
```

The load command and the printed SHA-256 cover the complete file, so they can be used
unchanged from the guest console or in NVRAM autoload:

```text
mcp.url=http://10.1.1.3:18080/my-module.module
mcp.sha256=<the digest printed above>
```

`--json` keeps the existing `module`, `size` and `sha256` keys; `size` and `sha256` are
over the complete file. With a manifest it adds `image_size` (the flattened payload),
`manifest_size` (the `I7MM` record length, excluding the footer), `footer_size` (always
16) and a `manifest` object in the same shape as the input document. The additions are
absent when no manifest is requested, so older consumers see exactly the old output.

### Explicit flags

The same manifest can be declared without a JSON file:

```bash
python3 tools/irix7-module.py \
  --package my-module \
  --name my-module --version 0.3.0 \
  --abi-major 1 --abi-minor 12 \
  --entry-offset 0 \
  --depends mesh.codec@1.0.0:<64 hex digits> \
  --export irix7.mesh.transport@1:128
```

`--depends` takes `name@version:sha256`; `--export` takes
`interface@version[:offset]` and may be repeated. The four identity fields
(`--name`, `--version`, `--abi-major`, `--abi-minor`) are required when any manifest
option is present, and `--manifest-json` cannot be combined with the explicit flags.
Mixing is rejected rather than silently merged.

With no manifest option the image is exactly the flattened payload, with no trailing
record or footer, and the printed digest is over that payload.

## Entry, context and export calls

A manager-aware module uses the SDK's `entry_with_context!` macro:

```rust
fn run(context: &module_sdk::Context, manager: *const module_sdk::ModuleContext)
    -> *const module_sdk::ModuleVtable { /* ... */ }

module_sdk::entry_with_context!(run);
```

The entry signature is
`const ModuleVtable *entry(const Services *, const ModuleContext *)`. The manager sets
`ModuleContext.manager` to its own state (usually a table of resolved exports) and the
kernel forwards the pointer unchanged. Legacy `entry!` modules keep the one-argument
`_start`; the extra argument is ignored. The tick callback keeps its
`(const Services *)` signature, so a module that needs the context on ticks stores the
pointer during entry.

An export table is interface-defined. A dependent does not receive an ambient kernel
pointer: the manager looks up `(interface, version)` with `module_find_export`, checks the
version, and passes the returned table (or a manager wrapper around it) through the
context it gives the dependent. The table lives inside the verified image and is revoked
when the module is unloaded.

## ABI mapping

The record's `abi major`/`abi minor` fields describe the module service table. The SDK's
table is append-only, so:

- `abi_major` is the table generation, currently `1`;
- `abi_minor` is `ABI_VERSION` from `abi/src/lib.rs`, currently `12`.

The host accepts a module when the major is equal and its minor is at least the module's
minor, matching the OS resolver's `Abi::is_compatible_with`. This is why a module built
for table 10 is stamped `1.10` rather than `10.0`: the latter would fail an equality test
against a host at table 12 even though every additive service is compatible.

The build tool reads `ABI_VERSION` from the crate rather than keeping a second copy, so a
service-table bump cannot silently leave the manifest behind. A manifest format change
does not bump `ABI_VERSION`; ABI 12 is for the loader services, and manifest v2 is a
separate format version.

## Relation to runtime loading

This is the build-time half of
[issue #217](https://github.com/irix7/os/issues/217). The SDK does not resolve, fetch,
load or execute a dependency. The ABI 12 loader performs the fetch/SHA-256/copy and
entry; the manager resolves the graph, loads dependencies before dependents, executes
each with a manager context and publishes export bindings. The OS `module-resolver`
remains the host-side resolver.

Version 1 of the manifest has exact versions only. Version ranges, a package-index
parser, signatures, slot allocation and a module manager are later slices.
`exports` records the interface name, version and table offset; the table contents are
interface-defined and the manager owns the call contract.

## OS changes still required

- `crates/module-resolver` must decode manifest v2 and expose `entry_offset` and export
  offsets. A v1-only decoder rejects v2 records as `UnsupportedVersion(2)`.
- The manager must add the `module_load`/`module_execute`/`module_find_export` calls and
  handle generation/stale-handle failures. It must not unload a dependency while a
  dependent holds a table.
- The kernel must implement ABI 12 (see [`module-loader.md`](module-loader.md)) and the
  release workflow must publish ABI 12 modules and the manager atomically.
- The package index has no `depends` field yet; the release slice adds exact dependency
  records.

## Tests

`testdata/manifest-v1.txt` and `testdata/manifest-minimal.txt` are canonical version 1
vectors produced by `module_resolver::footer::wrap` in irix7/os at `5a7696e`. They are
decode-only compatibility vectors: the encoder emits version 2.

`testdata/manifest-v2.txt` and `testdata/manifest-v2-minimal.txt` are the SDK's version 2
vectors with non-zero entry/export offsets and a 256-byte image. Both implementations
must reproduce the same record, footer and wrapped bytes:

```bash
cargo test -p irix7-module-manifest
python3 tools/tests/test_module_manifest.py
```

The Python tests also exercise the build tool end to end with a fake cargo and
`llvm-objcopy`, including the digest over the wrapped image, image-offset rejection and
the unchanged no-manifest path.
