# Provisional userland ABI

The SDK owns `irix7-user-abi` in `user-abi/`.  It is the single copy of the
**bring-up** user syscall numbers, open flags and C layouts inside the SDK.  The
OS still carries matching definitions in `crates/irix7-libc/src/sys.rs` and
`kernel/src/user.rs`; `tools/check-os-sync.py` compares them.

Version: **0** (`PROVISIONAL_ABI_VERSION`).  This is not the module ABI version
(10) and not the native capability ABI.  POSIX userland is the separate
[`irix7/relibc`](https://github.com/irix7/relibc) fork.

## Entry convention

- n32, big-endian, MIPS III, soft float.
- Syscall number in `$v0`.
- Arguments in `$a0`-`$a3`.
- Result in `$v0`; `-1` (`usize::MAX`) is the only error value.
- No `errno` syscall and no per-thread error state.

The kernel dispatches only in user mode (`Status.KSU`/`UX`).  A syscall in
kernel mode other than the console self-test is a fatal trap.

## Calls

| `$v0` | Name | Arguments | Result |
|---:|---|---|---|
| 1 | `SYS_WRITE` | `$a0` fd, `$a1` buffer, `$a2` length | bytes or `-1` |
| 2 | `SYS_EXIT` | `$a0` status | does not return |
| 3 | `SYS_MMAP` | `$a1` length (address/prot/flags ignored) | base or `-1` |
| 4 | `SYS_MUNMAP` | `$a0` address, `$a1` length | `0` (one-way heap) |
| 5 | `SYS_MPROTECT` | `$a0` address, `$a1` length, `$a2` protection | `0` (already RW) |
| 6 | `SYS_OPEN` | `$a0` path, `$a1` flags, `$a2` mode (ignored) | fd or `-1` |
| 7 | `SYS_CLOSE` | `$a0` fd | `0` or `-1` |
| 8 | `SYS_FCNTL` | `$a0` fd, `$a1` command, `$a2` argument | `0` (ignored) |
| 9 | `SYS_DUP` | `$a0` fd | new fd or `-1` |
| 10 | `SYS_GETCWD` | `$a0` buffer, `$a1` size | `0` or `-1` |
| 11 | `SYS_POLL` | `$a0` array, `$a1` count, `$a2` timeout | ready count or `-1` |
| 12 | `SYS_SIGNAL` | `$a0` signal, `$a1` handler | `0` (accepted, ignored) |
| 13 | `SYS_GETENTROPY` | `$a0` buffer, `$a1` length | `0`; buffer is zeroed |
| 14 | `SYS_CLOCK_GETTIME` | `$a0` clock id, `$a1` `Timespec *` | `0` or `-1` |
| 15 | `SYS_GETPID` | none | `1` |
| 16 | `SYS_READ` | `$a0` fd, `$a1` buffer, `$a2` length | bytes, `0` at EOF, `-1` |
| 17 | `SYS_FSTAT` | `$a0` fd, `$a1` `Stat *` | `0` or `-1` |
| 18 | `SYS_STAT` | `$a0` path, `$a1` `Stat *` | `0` or `-1` |
| 19 | `SYS_READDIR` | `$a0` fd, `$a1` index, `$a2` name, `$a3` length | type, `0` at end, `-1` |
| 20 | `SYS_NANOSLEEP` | `$a0` `Timespec *` | `0` or `-1` |
| 21 | `SYS_CHDIR` | `$a0` path | `0` or `-1` |

`SYS_READDIR` returns the entry type (`2` character device, `4` directory,
`8` regular file) and writes the NUL-terminated name.  It is an index-based
iterator, not `getdents`.

`SYS_CLOCK_GETTIME` treats clock id `0` as `CLOCK_REALTIME` (RTC/NTP; zero if
the date is unknown) and every other id as `CLOCK_MONOTONIC` (CP0 Count at
33 MHz).

`SYS_MMAP` returns zero-filled pages from a fixed 1 MiB window at
`0x2000_0000`.  It is a bump allocator: `SYS_MUNMAP` does not return pages.

## Open flags

| Name | Value | Bring-up behaviour |
|---|---:|---|
| `O_RDONLY` | 0 | accepted |
| `O_WRONLY` | 1 | accepted; mode is not enforced |
| `O_RDWR` | 2 | accepted; mode is not enforced |
| `O_CREAT` | `0o100` | accepted and ignored |
| `O_TRUNC` | `0o1000` | truncates `/hello.txt` and `/etc/motd` |
| `O_APPEND` | `0o2000` | accepted and ignored |

`O_TRUNC` is the only flag the kernel acts on.  There is no `O_EXCL`,
`O_DIRECTORY`, `O_NONBLOCK` or creation.

## `struct stat`

`#[repr(C)]`, 128 bytes, alignment 8:

| Offset | Field | Type |
|---:|---|---|
| 0 | `st_dev` | `u64` |
| 8 | `st_ino` | `u64` |
| 16 | `st_mode` | `u32` |
| 20 | `st_nlink` | `u32` |
| 24 | `st_uid` | `u32` |
| 28 | `st_gid` | `u32` |
| 32 | `st_rdev` | `u64` |
| 40 | `st_size` | `i64` |
| 48 | `st_blksize` | `i32` |
| 52 | `_pad` | `u32` |
| 56 | `st_blocks` | `i64` |
| 64 | `st_atim` | `[i64; 2]` |
| 80 | `st_mtim` | `[i64; 2]` |
| 96 | `st_ctim` | `[i64; 2]` |
| 112 | `spare` | `[u8; 16]` |

Mode constants: `S_IFMT = 0o170000`, `S_IFREG = 0o100000`, `S_IFDIR = 0o040000`,
`S_IFCHR = 0o020000`.  The bring-up namespace reports `0o100644` for files,
`0o040755` for directories and `0o020666` for devices.

## `struct timespec` and `struct pollfd`

`Timespec` is 16 bytes: `tv_sec: i64` at 0, `tv_nsec: i32` at 8, `_pad: u32` at
12.  `PollFd` is 8 bytes: `fd: i32` at 0, `events: i16` at 4, `revents: i16` at
6.  `POLLNVAL` (`0x20`) is reported for a closed descriptor; console and null
descriptors are always ready.

## Namespace

The kernel's bring-up namespace is fixed:

```text
/            directory
/etc         directory
/dev         directory
/dev/null    character device
/dev/tty     console (open returns fd 0)
/hello.txt   mutable ramfs file
/etc/motd    mutable ramfs file
```

`getcwd` and `chdir` accept `/`, `/etc` and `/dev`; relative paths resolve
against the current directory.  There is no `/bin`, no executable format
beyond this ELF loader and no way to create a file.

## Relationship to POSIX and the native ABI

- POSIX names in the runtime (`File`, `stat`, the `O_*` constants) are
  conveniences over these calls, not a claim of POSIX conformance.
- The native capability ABI is the kernel's internal object model; user
  programs will see POSIX through relibc, not these numbers.
- When relibc's `irix7` backend lands, the SDK runtime is retired or reduced to
  a boot probe.  It is not extended into a second libc.
