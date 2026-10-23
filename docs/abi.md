# Module ABI

The SDK owns the `module-abi` crate in `abi/`; the IRIX 7 OS consumes it at the pinned SDK
tag. The current ABI version is **10**.

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
