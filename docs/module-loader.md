# Module loader ABI (v12)

**Status:** SDK contract, host-tested C/Rust types. The OS kernel does **not** implement
these services yet. This page is the exact proposal a kernel and a downloadable manager
must satisfy; it does not claim that either exists.

## Why this breaks the freeze

The ROM-freeze rule says new runtime features ship as downloadable modules, not as
compressed-PROM code. A module manager cannot implement a loader by itself:

- it cannot reserve or mark executable arena memory;
- it cannot fetch an image, verify its digest and copy it into that memory as one
  operation;
- it cannot perform MIPS data-cache writeback and instruction-cache invalidate;
- it cannot publish a loaded-module table that survives its own replacement.

ABI 12 therefore appends a **mechanism** to the service table. This is an explicit,
reviewed exception to the freeze, not a general licence to grow the PROM:

| Stays in the compressed PROM | Stays in the downloadable manager |
|---|---|
| Arena geometry and use bits | Dependency graph and exact-version selection |
| Fetch, SHA-256 verification and cache-safe copy | Manifest decode and version/ABI policy |
| Entry call, cache maintenance and vtable/tick registration | Package-index parsing and releases |
| Loaded-handle table, exact identity/export lookup, unload | Signatures, key rotation and trust policy |
| — | Retry, rollback, refcount/unload decisions, which module runs |

The kernel verifies one image against one digest and records one identity. It does not
parse a manifest, resolve ranges, read a package index or decide which dependency is
acceptable. A failed load leaves no partial module visible.

## Service table append

The v11 fields at `0x00`-`0x50` are unchanged. ABI 12 appends these fields:

| Offset | Field | C signature |
|---|---|---|
| `0x54` | `module_arena` | `int32_t (*)(struct ModuleArena *out)` |
| `0x58` | `module_load` | `int32_t (*)(const struct ModuleLoad *request, uint64_t *out_handle)` |
| `0x5c` | `module_execute` | `int32_t (*)(uint64_t handle, uint32_t entry_offset, const struct ModuleContext *context, int32_t *out_status)` |
| `0x60` | `module_find` | `int32_t (*)(const uint8_t *name, uint32_t name_len, const uint8_t *version, uint32_t version_len, uint64_t *out_handle)` |
| `0x64` | `module_find_export` | `int32_t (*)(const uint8_t *interface, uint32_t interface_len, uint16_t version, struct ModuleExportBinding *out)` |
| `0x68` | `module_query` | `int32_t (*)(uint64_t handle, struct ModuleInfo *out)` |
| `0x6c` | `module_unload` | `int32_t (*)(uint64_t handle)` |

`size_of::<Services>()` is `0x70` (112 bytes) on n32; every existing field keeps its v11
offset. All structures below are `#[repr(C)]` native-endian memory layouts, and pointer
fields are 32-bit n32 addresses. Only the `I7MM` manifest record is little-endian.

## Arena

```c
struct ModuleArena {              // 32 bytes
    uint32_t base;                // 0x00, kseg0 address of region 0
    uint32_t length;              // 0x04, total reserved bytes
    uint32_t region_size;         // 0x08, bytes per region
    uint32_t region_count;        // 0x0c, at most MODULE_REGION_MAX (64)
    uint32_t used_lo;             // 0x10, bit i = region i is loaded
    uint32_t used_hi;             // 0x14, bit i = region 32+i is loaded
    uint32_t flags;               // 0x18, MODULE_ARENA_EXECUTABLE = bit 0
    uint32_t reserved;            // 0x1c, zero
};
```

Regions are contiguous and equally sized: region `i` is
`base + i * region_size .. + region_size`, and `length == region_size * region_count`.
The loader arena is a kernel-chosen region set inside the reserved module window; it does
not have to be the legacy 512 KiB/1.5 MiB A/B slots, which remain the standalone-module
path until the manager owns the arena. A module loaded through ABI 12 must fit one
region; a larger image stays in a legacy slot until a variable-region or relocatable
arena slice lands. `module_arena` returns `MODULE_OK` and fills the struct, or a
`MODULE_ERR_*` status.

## Loading

```c
struct ModuleLoad {               // 80 bytes
    const uint8_t *url;           // 0x00, immutable image URL in module memory
    uint32_t url_len;             // 0x04, at most MODULE_URL_MAX (511)
    uint8_t  sha256[32];          // 0x08, expected digest of the complete file
    const uint8_t *name;          // 0x28, exact module name
    uint32_t name_len;            // 0x2c, at most MODULE_NAME_MAX (63)
    const uint8_t *version;       // 0x30, exact module version
    uint32_t version_len;         // 0x34, at most MODULE_VERSION_MAX (31)
    const struct ModuleExport *exports; // 0x38, null when count is zero
    uint32_t export_count;        // 0x3c, at most MODULE_EXPORT_MAX (16)
    uint32_t region;              // 0x40, arena region index
    uint32_t flags;               // 0x44, MODULE_LOAD_REPLACE = bit 0
    uint32_t max_size;            // 0x48, 0 = region_size
    uint32_t reserved;            // 0x4c, zero
};

struct ModuleExport {             // 16 bytes
    const uint8_t *interface;     // 0x00, UTF-8 interface name
    uint32_t interface_len;       // 0x04, at most MODULE_INTERFACE_MAX (63)
    uint16_t version;             // 0x08
    uint16_t reserved;            // 0x0a, zero
    uint32_t offset;              // 0x0c, image-relative C table, 0 = no table
};
```

`module_load` performs one atomic transition:

1. validate the region, limits, pointers and offset alignment;
2. fetch `url` with a hard size bound (`max_size`, else `region_size`);
3. SHA-256 the **exact fetched bytes**, including any `I7MM` record and `I7MF` footer;
4. compare with `sha256`; a mismatch is `MODULE_ERR_DIGEST` and discards the buffer;
5. copy into the region and complete a data-cache writeback; the instruction cache is
   invalidated at entry;
6. copy the name, version and interface strings into the bounded table;
7. register the identity and exports, set the used bit and return a fresh handle.

The name, version, interface strings and export descriptors are copied before the call
returns; the caller may reuse those buffers. Any failure leaves the region untouched and
no lookup entry visible. `flags` bit 0 (`MODULE_LOAD_REPLACE`) replaces an occupied region
and increments the generation, so
older handles fail closed; without it an occupied region is `MODULE_ERR_BUSY`. A duplicate
`(name, version)` in another region or a duplicate `(interface, version)` export is
`MODULE_ERR_STATE`; resolution policy is the manager's, but the kernel will not create an
ambiguous table.

The URL is treated as immutable release data (HTTPS by default, explicit HTTP only on the
local rig). The digest, not the URL or the index, is the authority.

## Entering a module

```c
struct ModuleContext {            // 16 bytes
    uint32_t version;             // 0x00, MODULE_CONTEXT_VERSION (1)
    uint32_t flags;               // 0x04, manager-defined; zero unless documented
    const uint8_t *manager;       // 0x08, opaque manager state
    uint32_t reserved;            // 0x0c, zero
};
```

`module_execute(handle, entry_offset, context, out_status)`:

- rejects a stale handle, an unaligned offset or an offset outside the loaded image with
  `MODULE_ERR_STATE`/`MODULE_ERR_ARGUMENT`;
- performs a data-cache writeback and an instruction-cache invalidate for the region;
- calls the address `region_base + entry_offset` as
  `const struct ModuleVtable *entry(const struct Services *, const struct ModuleContext *)`;
- accepts a null vtable as success with status 0, and requires a non-null vtable to point
  inside the region;
- writes the vtable status to `out_status` (which may be null) and returns `MODULE_OK`;
- registers the vtable's optional `tick` callback and records `MODULE_STATE_ENTERED`.

The loader status and the module status are separate: the return value is always a
`MODULE_*` loader status, and `out_status` is the module's own result. A legacy
`_start(const Services *)` ignores the second register; a manager-aware entry reads it.
The tick callback keeps its existing `(const Services *)` signature, so a module that
needs the manager context on ticks must retain the pointer from its entry call. The
manager owns the context lifetime; it must outlive the loaded module.

## Lookup and exports

`module_find` matches the exact name bytes and version bytes copied at load; there are no
ranges or globs. `module_find_export` matches the exact interface bytes and `u16` version
and returns:

```c
struct ModuleExportBinding {      // 16 bytes
    uint64_t handle;              // 0x00, live owner
    const uint8_t *table;         // 0x08, absolute C table, or null
    uint16_t version;             // 0x0c, matched version
    uint16_t reserved;            // 0x0e, zero
};
```

The table address is computed as `region_base + ModuleExport.offset` after load, so no
symbol relocation, dynamic linker or ambient kernel pointer is involved. `offset = 0`
means the interface is declared without a table; the binding's `table` is null. The table
layout is interface-defined; the manager validates the interface version and passes the
binding to a dependent through its own context or manager service. The kernel never grants
an ambient pointer.

`module_query` returns the handle, base, image size, region, export count and state:

```c
struct ModuleInfo {               // 32 bytes
    uint64_t handle;              // 0x00
    uint32_t base;                // 0x08
    uint32_t image_size;          // 0x0c, bytes copied from the verified file
    uint32_t region;              // 0x10
    uint32_t export_count;        // 0x14
    uint32_t state;               // 0x18, MODULE_STATE_LOADED or _ENTERED
    uint32_t reserved;            // 0x1c, zero
};
```

`module_unload` unregisters the tick callback, removes identity/export entries, bumps the
generation and releases the region. It is explicit: the manager must prove no dependent
holds the handle or a table. Refcounting and automatic unload are **not** part of ABI 12.

Handles are `uint64_t` with the generation in the high 32 bits and the region in the low
32 bits; zero is never valid. Load, replace and unload all change the generation, so a
stale handle fails with `MODULE_ERR_STATE` instead of entering replaced bytes.

## Status, flags and limits

| Constant | Value | Meaning |
|---|---|---|
| `MODULE_OK` | `0` | Success |
| `MODULE_ERR_ARGUMENT` | `-1` | Null pointer, bad length, unaligned offset or wild range |
| `MODULE_ERR_REGION` | `-2` | Region index outside `region_count` |
| `MODULE_ERR_BUSY` | `-3` | Region occupied and no replace flag |
| `MODULE_ERR_DIGEST` | `-4` | SHA-256 mismatch |
| `MODULE_ERR_SIZE` | `-5` | Image too small/large for the entry or region |
| `MODULE_ERR_TRANSPORT` | `-6` | Fetch failed (DNS, TLS, HTTP, short read) |
| `MODULE_ERR_NOT_FOUND` | `-7` | Identity or export lookup missed |
| `MODULE_ERR_LIMIT` | `-8` | Name/version/interface/export-count limit exceeded |
| `MODULE_ERR_STATE` | `-9` | Stale handle, missing module or invalid entry offset |

`MODULE_LOAD_REPLACE = 1`, `MODULE_ARENA_EXECUTABLE = 1`,
`MODULE_STATE_LOADED = 1`, `MODULE_STATE_ENTERED = 2`,
`MODULE_CONTEXT_VERSION = 1`, `MODULE_OFFSET_ALIGN = 4`, `MODULE_REGION_MAX = 64`.

Bounds: name 63 bytes, version 31, interface 63, exports per module 16, URL 511. These are
loader-table limits, separate from the manifest's 64 KiB record bound. Strings need not be
NUL-terminated; lengths are explicit. Every pointer is validated against the module arena
before the kernel reads it.

## Manifest v2 coordination

Manifest version 2 adds the two fields the loader needs:

```json
{
  "format": 2,
  "name": "mesh.transport",
  "version": "1.2.3",
  "abi_major": 1,
  "abi_minor": 12,
  "entry_offset": 0,
  "depends": [],
  "exports": [
    { "interface": "irix7.mesh.transport", "version": 1, "offset": 128 }
  ]
}
```

- `entry_offset` is the image-relative manager entry passed to `module_execute`. `0` is a
  legacy `_start(const Services *)`; a non-zero offset must be 4-byte aligned and inside
  the flattened payload.
- each `exports[].offset` is the image-relative C table copied into the `ModuleExport`
  passed to `module_load`; `0` means interface-only.
- version 1 manifests remain decodable: their entry and export offsets are zero, so they
  describe a legacy standalone module.
- dependency name/version/SHA-256 rules are unchanged. The manager resolves the graph,
  loads dependencies first, executes them with a manager context and only then enters the
  dependent.

The manager must pass the manifest's exact name, version and export list to
`module_load`; the kernel records the caller's values and does not parse the manifest to
cross-check them. The manifest remains trailing data for the kernel. The manager parses
it, chooses the entry and export offsets and drives the loader; the kernel never opens
it.

## Failure, trust and lifetime

- The digest covers image + manifest + footer. There is no unverified load path.
- A load is all-or-nothing; a failure leaves no handle, region bit or lookup entry.
- A dependency is loaded and verified before its dependent is entered. There is no
  partial execution of a graph.
- Module code runs in kernel mode. The digest is the trust boundary; this ABI is not a
  sandbox. A manager bug is a kernel bug, as with every module today. The manager must
  not modify a loaded region after `module_load`; `module_execute` does not re-hash it.
- There is no automatic unload or refcount in ABI 12. The manager decides lifetime and
  must not unload while a dependent can call an export.
- A generation mismatch on execute/query/unload fails closed.

## Not in ABI 12

- Signature/key verification (a later trust-store slice; the SHA-256 digest is mandatory).
- Version ranges or a solver (the manager and `module-resolver`).
- Package-index parsing or a release workflow change.
- Relocatable images; entries and export tables are addressed by image-relative offsets in
  a fixed region.
- Automatic dependency loading, retries, rollback or refcounting.
- A Wasm app loader: this ABI is for flat n32 images. Wasm guests remain embedded data in
  a host module ([`wasm-apps.md`](wasm-apps.md)).

## OS changes still required

1. Implement the seven services and the arena/reserved-window policy in the kernel, with
   the cache maintenance and the loaded-module table. This is the PROM growth the freeze
   exception authorises.
2. Move the OS pin to the SDK release that contains ABI 12 and construct the larger
   `Services` table; v11 modules continue to run unchanged.
3. Add manifest v2 decode (entry and export offsets) to `crates/module-resolver` and the
   manager module; v1-only readers reject v2 with `UnsupportedVersion(2)`.
4. Replace build-time bundling in the manager with `module_load`/`module_execute`/
   `module_find_export` calls, in dependency order.
5. Update `docs/spec/modules.md` with the implemented loader, arena and freeze exception,
   and publish the manager and its dependency set atomically.
6. Test on iris: a two-module graph loaded from exact digests, failure injection for a bad
   digest, stale handle after replace and unload, plus the existing M1/M2 markers unchanged
   when no dependency is declared.

## Tests in the SDK

`cargo test -p module-abi -p irix7-module-sdk` checks the n32 `Services` offsets and the
C layouts (compiled for the mips target in the module build), the status set, argument
rejection in the Rust wrappers and the loader status/module-status separation.
`cargo test -p irix7-module-manifest` and `python3 tools/tests/test_module_manifest.py`
check manifest v2 encode/decode, v1 compatibility and image-offset validation.
