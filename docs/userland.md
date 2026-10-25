# Userland SDK

Status: **bring-up**.  The SDK builds and validates static native n32 user images
on the host.  The images have not been run on an Indy by this SDK.

This document defines the userland surface and, just as importantly, what it is
not.  The production POSIX layer is the separate
[`irix7/relibc`](https://github.com/irix7/relibc) fork; it is not in this
repository and cannot be built with `irix7-user-sdk`.

## 1. Two targets, two execution models

| | Kernel module | User program |
|---|---|---|
| Crate | `irix7-module-sdk` | `irix7-user-sdk` |
| Target JSON | `targets/mips3-sgi-irix7-n32.json` | `targets/mips3-sgi-irix7-native-n32.json` |
| Linker | `linker/module.ld` | `linker/user.ld` |
| Output | flat binary | ELF32 executable + raw image |
| Load address | `0x8840_0000` (kseg0) | `0x1000_0000` (kuseg) |
| Mode | kernel mode | user mode |
| Trust | SHA-256 digest, no sandbox | one-way ELF load, `write`/`exit` syscalls |
| Entry | `_start(*const Services)` returns a vtable | `_start` calls `__irix7_user_main(&Context) -> i32` |
| Lifetime | until unregistered, reset or poweroff | until the exit syscall |
| Namespace | kernel service table | console, fixed bring-up descriptors |

A module is **not** a process.  It shares the kernel address space, can call the
service table directly and must not assume an MMU boundary.  A user image has
its own `PT_LOAD` mappings and a four-page stack, but the bring-up kernel has no
process table, address-space isolation or scheduler for it.

The two targets are deliberately not merged:

- `mips3-sgi-irix7-n32.json` leaves `os` unset and has no Unix family; it is the
  freestanding kernel/module target with `crt-objects-fallback = false`.
- `mips3-sgi-irix7-native-n32.json` declares `os = "irix7"`, the Unix family,
  `dynamic-linking = false`, `position-independent-executables = false`,
  `has-thread-local = false` and `default-uwtable = false`.  Those fields make
  the Rust `std`/`libc` cfg surface describe IRIX 7 rather than an unknown
  operating system.
- Both share `arch = mips64`, `abi = n32`, `cpu = mips3`, big-endian,
  `+soft-float`, `+noabicalls`, `relocation-model = static` and the same
  `data-layout`.  The user target is a copy of the OS target, not a second ABI
  design.

## 2. Boundary: bring-up runtime, not POSIX

`irix7-user-sdk` is a **static native bring-up runtime**.  It provides:

- `_start`, a `Context` over the loader-built stack, and `exit`;
- `no_std` console output (`print!`, `println!`);
- argument and environment access as byte slices;
- descriptor wrappers (`File`, `Dir`, `stat`, `getcwd`, `chdir`);
- clocks and a busy-wait `sleep`.

It does **not** provide:

- POSIX `errno`, `fork`, `exec`, `posix_spawn`, `waitpid`, process groups;
- threads, TLS, `setjmp`/`longjmp`, unwinding;
- signals, `termios`, job control, pipes, sockets;
- a general filesystem, permissions, `openat`, `lseek`, `mmap` file mappings;
- an allocator, `std`, collections, a dynamic linker or shared libraries.

The kernel's provisional syscall ABI is the contract in
[`user-abi.md`](user-abi.md).  It is the **bring-up** ABI used by
`crates/irix7-libc` and `user/libc` in the OS, not the native capability ABI in
`docs/spec/native-abi.md` and not a POSIX ABI.  When relibc's IRIX 7 backend
lands, it replaces this runtime rather than extending it.

## 3. Linker layout

`linker/user.ld` fixes the contract:

```text
0x1000_0000  .text      (entry first; KEEP(.text._start))
             .rodata
             .data      (.data, .sdata, .got)
             .bss       (NOLOAD; zero-filled by the loader)
```

`OUTPUT_FORMAT("elf32-ntradbigmips")`, `ENTRY(_start)`.  The target is
`+noabicalls`, so code uses absolute addressing and `_start` does not set `$gp`.
The linker discards `.MIPS.abiflags`, `.reginfo`, `.pdr`, `.comment`,
`.gnu.attributes`, `.mdebug*`, `.options*` and `.eh_frame`; their absence keeps
the ELF under the loader window and avoids a synthetic `PT_MIPS_*` segment.

`_start` is 32 bytes of MIPS III assembly in `user-sdk/src/rt.rs`:

```asm
move    $a0, $sp
jal     __irix7_user_main
nop
move    $a0, $v0
li      $v0, 2          # SYS_EXIT
syscall
```

`irix7-user.py` passes `--undefined=_start` to the linker so the runtime object
containing `_start` is pulled from the rlib even though the entry symbol is the
only reference to it.

## 4. Loader contract

The kernel's `userelf` path:

1. initialises SCSI target 2 and reads up to **262144 bytes** from LBA 0 in
   64 KiB chunks;
2. parses the image with `irix-elf` and requires `Abi::N32`;
3. maps every `PT_LOAD` range into kuseg with 4 KiB pairs;
4. copies `p_filesz` bytes through the uncached kseg1 alias, zero-fills the
   rest of `p_memsz`, and invalidates the instruction cache;
5. maps a four-page stack at `0x7FF0_0000`;
6. builds `argc`, `argv`, `envp` and the strings on the stack, sets `$sp` and
   enters at `e_entry`.

The loader currently supplies `argv[0] = "hello"`, `PATH=/` and `HOME=/`
regardless of the image name.  It records the image, stack and heap ranges for
the `write`/`read` bounds check.  The anonymous heap is a fixed 1 MiB window at
`0x2000_0000`, allocated on the first `mmap`; the SDK does not use it by
default.

The image is entered automatically after boot if target 2 is present.  The
telnet `userelf` command reloads it.  A user exit returns to the console and
prints the status.

## 5. Building

From an SDK checkout:

```bash
python3 tools/irix7-user.py --package user-hello-sdk
```

The default manifest is `examples/user-hello/Cargo.toml`, the default target is
`targets/mips3-sgi-irix7-native-n32.json`, and the default linker is
`linker/user.ld`.  For another package:

```bash
python3 tools/irix7-user.py \
  --manifest-path examples/user-probe/Cargo.toml \
  --package user-probe-sdk \
  --output /tmp/user-probe.elf \
  --image /tmp/user-probe.raw
```

The tool:

- reads the cargo target directory from `cargo metadata`;
- sets `CARGO_TARGET_MIPS3_SGI_IRIX7_NATIVE_N32_RUSTFLAGS` to
  `-Clink-arg=-T…/linker/user.ld -Clink-arg=--undefined=_start`;
- builds with `-Z build-std=core -Z json-target-spec --release`;
- validates the ELF before writing anything;
- pads the image to exactly `--image-size` bytes (default 262144);
- prints the ELF, image, entry, section names, SHA-256 and sizes, or JSON.

Use `--debug` for the debug profile and `--no-image` for an ELF-only check.  A
debug ELF normally exceeds 256 KiB once symbols are included; the validator
will reject it.

## 6. What the validator checks

`tools/irix7-user.py` rejects an image unless all of the following hold:

| Check | Requirement |
|---|---|
| ELF class/data | `ELFCLASS32`, `ELFDATA2MSB`, `EV_CURRENT` |
| Type/machine | `ET_EXEC`, `EM_MIPS` |
| ABI flags | ABI field zero, `EF_MIPS_ABI2` set, MIPS III arch, PIC bit clear |
| Program headers | at least one non-empty `PT_LOAD`; ranges inside the file; `p_filesz <= p_memsz`; 32-bit address range; valid power-of-two alignment |
| Static | no `PT_INTERP`, `PT_DYNAMIC` or `PT_TLS`; no `.dynsym` |
| Entry | inside an executable `PT_LOAD` and inside `.text` |
| Sections | `.text` allocated/executable/not writable; `.rodata` allocated/not writable; every allocated section covered by a `PT_LOAD` |
| Symbols | `.symtab` present; no `SHN_UNDEF` symbol other than index 0 |
| Size | ELF and raw image no larger than the loader window |

The Python parser is in `tools/irix7-user.py`; `tools/tests/test_user_image.py`
runs it against a built ELF, corrupts individual fields to prove the negative
paths, and checks the raw image is exactly 262144 bytes with the ELF at offset
0 and zeros after it.  It does not execute the image.

## 7. 256 KiB target-2 image

The kernel reads at most 256 KiB, and QEMU/SCSI images are easier to attach at a
fixed size, so the tool writes:

```text
image[0 .. len(elf)]        = ELF
image[len(elf) .. 262144]   = 0x00
```

The raw image is not an ISO, a filesystem or a boot sector.  Attach it as SCSI
target 2; the ELF starts at LBA 0.  The SHA-256 in the JSON output covers the
padded image, not just the ELF.

## 8. Runtime API

```rust
#![no_std]
#![no_main]

use user_sdk::{entry, panic_handler, println, Context};

fn run(context: &Context) -> i32 {
    println!("argc={}", context.argc());
    for argument in context.args() {
        // argument is &[u8], not necessarily UTF-8
    }
    if let Some(path) = context.env(b"PATH") {
        // path is the bytes after '='
    }
    0
}

entry!(run);
panic_handler!();
```

- `Context::from_stack` validates `argc`, every `argv`/`envp` pointer and the
  NUL terminators.  It rejects a malformed stack; `entry!` then returns 127.
- `Context::args()`, `env_vars()` and `arg(index)` return `&'static [u8]`.
  The references live until the image exits because the kernel stack is not
  reused.
- `fs::File` closes on `Drop`; `fs::Dir` iterates by index.
- `sys` wrappers return `-1`, `None` or `false` on failure.  There is no
  `errno`; the kernel has none.
- `time::sleep` busy-waits in the kernel.  It is not a scheduler sleep.

`examples/user-hello` exercises console, arguments, environment and a `.bss`
guard.  `examples/user-probe` exercises `/hello.txt`, `/etc/motd`, `getcwd`,
directory iteration, realtime, monotonic time and `nanosleep`.

## 9. Keeping the SDK and OS copies in sync

The SDK is the **normative** copy of the user target and the shared user ABI.
The OS currently carries its own copies because it predates this SDK surface:

| Contract | SDK | OS copy |
|---|---|---|
| User target | `targets/mips3-sgi-irix7-native-n32.json` | `targets/mips3-sgi-irix7-native-n32.json` |
| Linker base | `linker/user.ld` | `user/hello/linker.ld` |
| Syscall numbers | `user-abi/src/lib.rs` | `crates/irix7-libc/src/sys.rs` |
| `Stat` layout | `user-abi/src/lib.rs` | `crates/irix7-libc/src/sys.rs`, `kernel/src/user.rs` |
| Loader window | `tools/irix7-user.py` | `kernel/src/user.rs` (`ELF_SIZE`) |
| SCSI target | docs and tool | `kernel/src/user.rs` (`TARGET`) |

Run the parity gate before changing a shared constant:

```bash
python3 tools/check-os-sync.py --os /mnt/europa/os
```

It compares the target JSON field by field, the 21 `SYS_*` numbers, the field
sequence of all three `Stat` definitions, the linker format/base, the 256 KiB
window and target 2.  It exits 2 if no OS checkout is supplied.

The intended follow-up is for the OS to consume the SDK directly:

1. delete `targets/mips3-sgi-irix7-native-n32.json` from the OS or generate it
   from the SDK, and build `user/hello` with the SDK target;
2. re-point `crates/irix7-libc/src/sys.rs` at `irix7-user-abi` instead of
   repeating the numbers and layouts;
3. move the user linker to the SDK and have `user/hello` use it;
4. keep `kernel/src/user.rs`'s `ELF_SIZE`/`TARGET` assertions in the parity
   script until the loader table is part of a shared boot/user contract.

Until then, `check-os-sync.py` is the guard.  The public CI job cannot see the
OS repository, so it runs the build and image tests only; the sync check is a
local/release step.  A release tag (`v0.4.0` for the first userland release)
pins the SDK side for OS consumers.

## 10. Limitations and open items

- **Not run on hardware by the SDK.**  The OS has run a different static
  `std` image; this SDK runtime has only host build/link/validation coverage.
- **No allocator.**  Programs use fixed buffers.  A future `alloc` feature can
  build on `sys::mmap`, but the bring-up heap is one-way and 1 MiB.
- **No runtime on `std`.**  `std` needs the patched `rust-src` and the
  42-symbol shim in the OS; that path is separate from `irix7-user-sdk`.
- **Provisional syscall ABI.**  Numbers, `Stat` and `Timespec` are frozen only
  by `user-abi` and the parity script.  There is no ABI version at entry, no
  `errno`, no capability handle and no process id beyond the fixed value 1.
- **Single process, single thread, one address space.**  The loader frees the
  image and stack on exit; there is no `waitpid` for anyone to observe.
- **Fixed namespace and fixed argv[0].**  The kernel's ramfs fixture is tiny and
  the loader always supplies `"hello"`, `PATH=/` and `HOME=/`.
- **Console only.**  No `read` from the console is provided by the SDK, no
  `termios`, no line discipline and no input events.
- **No relocation or dynamic linking.**  The image is static `ET_EXEC`; keep
  it under 256 KiB including section headers and symbols.

## 11. CI

The public workflow:

1. formats and clippies `module-abi`, `irix7-module-sdk`, `irix7-user-abi` and
   `irix7-user-sdk` on the host;
2. builds the three module examples with `tools/irix7-module.py` and checks the
   flat images;
3. builds `examples/user-hello` and `examples/user-probe` with
   `tools/irix7-user.py`;
4. runs `tools/tests/test_user_image.py` against both ELF/image pairs.

It does not run QEMU or the Indy, and it does not fetch the OS repository.
