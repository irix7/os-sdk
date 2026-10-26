# Module manifests and runtime dependencies

A module image may carry a **version 1 manifest** after its flattened payload. The
manifest names the module, states the exact version and service-table ABI it was
built for, declares exact runtime dependencies with their SHA-256 digests, and
lists the interfaces it exports.

This document is the SDK's build-time surface for the format. The OS
[`module-resolver`](https://github.com/irix7/os/tree/5a7696e/crates/module-resolver)
implementation is the host-side resolver and the authority for the wire format;
[`docs/spec/modules.md` sections 8.1 and 8.2](https://github.com/irix7/os/blob/5a7696e/docs/spec/modules.md)
are the contract. The SDK and the OS must produce identical bytes, which the
cross-implementation vectors in [`testdata/`](../testdata) enforce.

The kernel does **not** read or validate a manifest today. A manifest is trailing
data after the image, so the entry point remains at offset 0 and the module
service ABI is unchanged. A module image without a manifest remains a valid
standalone module; the build tool preserves that behaviour when no manifest
option is given.

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
SHA-256 of the **complete** file — image, record and footer — is the integrity
boundary and the digest used by the `module` command and the NVRAM autoload
keys. The footer has no checksum of its own.

The record is little-endian and identical to the OS encoder:

```text
I7MM | u16 format=1 | u16 abi major | u16 abi minor
     | u16 name length | UTF-8 name
     | u16 version length | UTF-8 version
     | u16 dependency count
     | (u16 name length, name, u16 version length, version, 32-byte SHA-256)*
     | u16 export count
     | (u16 interface length, interface, u16 export version)*
```

Records are limited to 64 KiB. There is no CRC: the image digest covers the
record. Decoders reject bad magic, an unsupported format or footer version, a
non-zero reserved field, truncation, trailing bytes, invalid UTF-8, an
out-of-range footer and an oversized record. A present but malformed footer is
an error, never an older standalone image.

## Authoring a manifest in JSON

Create `manifest.json` beside the crate:

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
      "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
    }
  ],
  "exports": [
    { "interface": "irix7.mesh.transport", "version": 1 }
  ]
}
```

| Field | Required | Meaning |
|---|---|---|
| `format` | no | Manifest format. Only `1` is implemented; the default is `1`. |
| `name` | yes | Stable module name, `A-Z a-z 0-9 . _ + -`. |
| `version` | yes | Exact version, same character set as `name`. Ranges are not supported. |
| `abi_major` | yes | Service-table generation. The only generation this SDK builds is `1`. |
| `abi_minor` | yes | Service-table revision, at most the SDK's `ABI_VERSION`. |
| `depends` | no | Exact dependencies; each needs `name`, `version` and `sha256`. |
| `exports` | no | Exported interfaces; each needs `interface` and a `u16` `version`. |

Unknown keys are rejected so a typo such as `dependencies` cannot be ignored.
`depends` and `exports` default to empty lists.

`sha256` is 64 hexadecimal digits over the dependency's whole image, including
its own manifest and footer if it has one. Use the digest printed when that
dependency was built. The release side of
[issue #217](https://github.com/irix7/os/issues/217) will publish the same value;
the package index has no `depends` field yet.

The SDK applies these build-time rules on top of the wire format:

- names, versions and interfaces must be non-empty and use `A-Z a-z 0-9 . _ + -`;
- a dependency may appear once; two versions of the same dependency name are a
  conflict in the v1 exact-version model and are rejected;
- an export may not repeat the same `interface` and `version` pair (two versions
  of one interface are allowed);
- `abi_major` must be `1` and `abi_minor` must not be newer than the service
  table in [`abi/src/lib.rs`](../abi/src/lib.rs);
- the encoded record must fit the 64 KiB bound and every length field must fit a
  `u16`.

## Building with a manifest

```bash
python3 tools/irix7-module.py \
  --package my-module \
  --manifest-json my-module/manifest.json
```

The tool builds and flattens the image as before, appends the record and footer,
then prints the final size and digest:

```text
module: /home/you/os-sdk/my-module.module
size:   4622 bytes
sha256: <64 hex digits over image + manifest + footer>
image:  4450 bytes (flattened image before the manifest)
manifest: mesh.transport@1.2.3 abi 1.11, 1 dependency, 1 export (record 156 bytes, footer 16 bytes)
load:   module http://<host>:<port>/my-module.module <64 hex digits>
```

The load command and the printed SHA-256 cover the complete file, so they can be
used unchanged from the guest console or in NVRAM autoload:

```text
mcp.url=http://10.1.1.3:18080/my-module.module
mcp.sha256=<the digest printed above>
```

`--json` keeps the existing `module`, `size` and `sha256` keys; `size` and
`sha256` are over the complete file. With a manifest it adds `image_size` (the
flattened payload), `manifest_size` (the `I7MM` record length, excluding the
footer), `footer_size` (always 16) and a `manifest` object in the same shape as
the input document. The additions are absent when no manifest is requested, so
older consumers see exactly the old output.

### Explicit flags

The same manifest can be declared without a JSON file:

```bash
python3 tools/irix7-module.py \
  --package my-module \
  --name my-module --version 0.3.0 \
  --abi-major 1 --abi-minor 11 \
  --depends mesh.codec@1.0.0:<64 hex digits> \
  --export irix7.mesh.transport@1
```

`--depends` takes `name@version:sha256` and `--export` takes
`interface@version`; both may be repeated. The four identity fields
(`--name`, `--version`, `--abi-major`, `--abi-minor`) are required when any
manifest option is present, and `--manifest-json` cannot be combined with the
explicit flags. Mixing is rejected rather than silently merged.

With no manifest option the image is exactly the flattened payload, with no
trailing record or footer, and the printed digest is over that payload.

## ABI mapping

The record's `abi major`/`abi minor` fields describe the module service table.
The SDK's table is append-only, so:

- `abi_major` is the table generation, currently `1`;
- `abi_minor` is `ABI_VERSION` from `abi/src/lib.rs`, currently `11`.

The host accepts a module when the major is equal and its minor is at least the
module's minor, matching the OS resolver's `Abi::is_compatible_with`. This is why
a module built for table 10 is stamped `1.10` rather than `10.0`: the latter
would fail an equality test against a host at table 11 even though every
additive service is compatible.

The build tool reads `ABI_VERSION` from the crate rather than keeping a second
copy, so a service-table bump cannot silently leave the manifest behind. There
is no service ABI version bump for manifests: the table layout, `Services`
offsets and `ModuleVtable` are untouched.

## Relation to runtime loading

This is the build-time half of
[issue #217](https://github.com/irix7/os/issues/217). The SDK does not resolve,
fetch, load or execute a dependency. Dependency digests are checked for shape
only; the OS resolver verifies each image, rejects missing modules, digest
mismatches, ABI mismatches and cycles, and returns a deterministic order with
dependencies before dependents.

Version 1 has exact versions only. Version ranges, a package-index parser,
signatures, export function-table addresses, slot allocation and a module
manager are later slices and are not part of this format. `exports` records the
interface name and version; publishing the C table is a loader concern.

## Tests

`testdata/manifest-v1.txt` and `testdata/manifest-minimal.txt` are canonical
vectors produced by `module_resolver::footer::wrap` in irix7/os at `5a7696e`.
Both implementations must reproduce the same record, footer and wrapped bytes:

```bash
cargo test -p irix7-module-manifest
python3 tools/tests/test_module_manifest.py
```

The Python tests also exercise the build tool end to end with a fake cargo and
`llvm-objcopy`, including the digest over the wrapped image and the unchanged
no-manifest path.
