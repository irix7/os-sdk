# Module ABI

The SDK owns the `module-abi` crate in `abi/`; the IRIX 7 OS consumes it at the pinned SDK
tag. The current ABI version is **13**.

This document covers **kernel modules only**.  Static user images use the
separate provisional userland ABI in [`user-abi.md`](user-abi.md); the two are
not interchangeable.

The service table is `#[repr(C)]`, append-only and passed to `_start`. New services are
added at the end and `ABI_VERSION` is bumped; existing offsets never move. The kernel and
every module link the same crate, and compile-time assertions pin the n32 offsets.

| Offset | Service |
|---|---|
| 0x00 | `version` |
| 0x04 | `write` (console) |
| 0x08 | `clock_ms` |
| 0x0C | `alloc` |
| 0x10 | `poll` |
| 0x14 | `tcp_listen` |
| 0x18 | `tcp_state` |
| 0x1C | `tcp_recv` |
| 0x20 | `tcp_send` |
| 0x24 | `tcp_close` |
| 0x28 | `tcp_abort` |
| 0x2C | `console_read` |
| 0x30 | `console_total` |
| 0x34 | `reset` |
| 0x38 | `poweroff` |
| 0x3C | `status` |
| 0x40 | `config_get` |
| 0x44 | `info` (status/net/mem/time) |
| 0x48 | `screen_read` (BGR shadow) |
| 0x4c | `block_read` (512-byte SCSI sector) |
| 0x50 | `block_write` (512-byte SCSI sector) |
| 0x54 | `module_arena` (query the bounded loader arena) |
| 0x58 | `module_load` (fetch, verify and copy a flat image) |
| 0x5c | `module_execute` (enter a loaded image with a manager context) |
| 0x60 | `module_find` (exact module name and version) |
| 0x64 | `module_find_export` (exact interface and version) |
| 0x68 | `module_query` (region, base, size and state of a handle) |
| 0x6c | `module_unload` (release a region) |
| 0x70 | `tcp_connect` (IPv4 network order + port) |

ABI 10 added `block_read`; ABI 11 added `block_write`; ABI 12 added the module-loader and
arena services; ABI 13 adds `tcp_connect`. `size_of::<Services>()` is `0x74` on n32.

[`examples/block`](../examples/block) is the reference for `block_read`: it reads sector 0
of target 1 and prints the volume-header magic. It checks the table version before the
first call and reports an older table with a bounded `block: ABI 10 required` message.

The full kernel-side contract, including the vtable, tick callback, TCP slot semantics,
flat-image layout and cache maintenance, is in the OS
[module specification](https://github.com/irix7/os/blob/main/docs/spec/modules.md).

A module entry returns a `*const ModuleVtable`:

```c
struct ModuleVtable {
    int32_t status;                                          // 0 = success
    int32_t (*tick)(const struct Services *services);        // optional
};
```

The kernel calls `tick` from its idle loop at most every 10 ms. Return `0` to stay
registered and non-zero to stop. The vtable must live inside the verified image.

A manager-aware entry has one extra argument:

```c
const struct ModuleVtable *entry(const struct Services *services,
                                 const struct ModuleContext *manager);
```

Legacy one-argument modules ignore the second register. The SDK's `entry!` macro keeps
the legacy signature; `entry_with_context!` takes a `*const ModuleContext` as its second
argument. The kernel passes the manager pointer through unchanged.

## ABI 12 module loader: an explicit ROM-freeze exception

ABI 12 is the first exception to the "runtime features ship as modules" rule. A
downloadable manager cannot reserve executable memory, fetch and verify an image into it,
or perform MIPS cache maintenance from module code. The append at offsets `0x54`-`0x6c`
therefore puts the **mechanism** in the compressed PROM:

- arena geometry (`base`, `length`, `region_size`, `region_count`, used bits);
- a bounded load: immutable URL, expected SHA-256, region, identity and export list;
- entry at an image-relative offset with a manager-provided opaque context;
- exact identity/export lookup, query and unload of a loaded handle.

The **policy** stays in the downloadable manager module: dependency resolution, manifest
and version rules, signatures and key policy, package-index parsing, retry/rollback,
lifetime/refcount decisions and which module to load. The kernel does not parse a
manifest, resolve a graph or trust a package index; it verifies one image against one
digest and records one identity. The ABI, layouts, statuses, trust model and remaining
OS work are in [`module-loader.md`](module-loader.md).

The manager must implement this surface end to end before any release claims it. The OS
kernel at the pinned SDK tag does **not** implement ABI 12 yet; this repository defines
the contract and the host-tested Rust/C types only.

A module built against ABI 12 checks `services.version >= 12` in `Context::new` and
refuses to run on an older table. A v11 module still runs on a v12 kernel because the
existing offsets and signatures are unchanged. Do not bump `ABI_VERSION` again for a
manifest format change.

## Module manifests

A flat image may append a version 2 `I7MM` manifest record and an `I7MF` footer; see
[`module-manifest.md`](module-manifest.md). Version 1 records remain decodable for older
images. The record's `abi major` and `abi minor` are this service table's generation
(`1`) and `ABI_VERSION` (`12`), not a second service-table version. A manifest is
trailing data: it does not move `_start` from offset 0, the `Services` offsets and
`ModuleVtable` are unchanged, and the kernel does not read it. **Do not bump
`ABI_VERSION` for a manifest format change.** ABI 12 was bumped for the loader service,
which is a separate addition.
