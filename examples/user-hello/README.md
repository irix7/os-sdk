# `user-hello`

Console, argument, environment and BSS example for `irix7-user-sdk`.

```bash
python3 tools/irix7-user.py --package user-hello-sdk
```

The image prints the greeting, every argument and the `PATH`/`HOME` values,
checks that the loader zeroed an 8192-byte `.bss` guard, and exits 0.  It is a
host build/link test in CI; the SDK has not run it on an Indy.

This is not a POSIX program.  See [`../../docs/userland.md`](../../docs/userland.md).
