# `user-probe`

Filesystem, directory and clock probe for `irix7-user-sdk`.

```bash
python3 tools/irix7-user.py \
  --manifest-path examples/user-probe/Cargo.toml \
  --package user-probe-sdk
```

It stats `/hello.txt`, reads `/etc/motd`, prints the working directory and the
root directory entries, reads the realtime and monotonic clocks, sleeps for one
millisecond and exits non-zero if an expected fixture is missing.  The namespace
is the kernel's bring-up fixture, not a general filesystem.

It is a host build/link test in CI; the SDK has not run it on an Indy.  See
[`../../docs/userland.md`](../../docs/userland.md).
