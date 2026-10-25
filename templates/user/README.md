# IRIX 7 user program template

A copyable starting crate for a **static native n32 user image**.  It is not a
kernel module and it is not a POSIX application: it links `irix7-user-sdk`, the
provisional syscall/runtime crate.

```bash
cargo generate --git https://github.com/irix7/os-sdk --subfolder templates/user --name my-user-program
```

Build it from an SDK checkout (the tool owns the target JSON, linker flags,
ELF validation and the 256 KiB target-2 image):

```bash
python3 tools/irix7-user.py \
  --manifest-path ../my-user-program/Cargo.toml \
  --package my-user-program \
  --linker ../my-user-program/linker.ld \
  --output ../my-user-program/my-user-program.elf
```

The result is a big-endian n32 `ET_EXEC` image linked at `0x1000_0000` plus a
262144-byte raw SCSI image at `my-user-program.raw`.  Attach the raw image as
SCSI target 2; the kernel enters it after boot and the telnet `userelf` command
reloads it.

The dependency is pinned to `v0.4.0`, the first release with `irix7-user-sdk`.
While that tag is unreleased, a local trial can replace the dependency with
`{ path = "/path/to/os-sdk/user-sdk" }` or `branch = "main"`.  See
[`../../docs/userland.md`](../../docs/userland.md) for the boundary and the
current limitations.
