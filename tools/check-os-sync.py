#!/usr/bin/env python3
"""Check the SDK user target and provisional ABI against an OS checkout.

The SDK owns the userland target, the linker base, the loader-window size and
the provisional ABI constants.  While the OS still carries its own copies of
these files, this script is the parity gate:

    python3 tools/check-os-sync.py --os /mnt/europa/os

It compares:

- `targets/mips3-sgi-irix7-native-n32.json` field by field;
- the `SYS_*` numbers in `user-abi/src/lib.rs` with
  `crates/irix7-libc/src/sys.rs`;
- the `Stat`/`KernelStat` field sequence with the two OS definitions;
- the linker format and load address with `user/hello/linker.ld`;
- the 256 KiB image size and SCSI target 2 with `kernel/src/user.rs`.

Exit status is 0 when everything matches, 1 on drift and 2 when the OS tree is
not supplied.  The script is not part of the public CI job because the OS source
is a separate repository; run it locally before changing a shared constant.
"""

import argparse
import json
import os
import pathlib
import re
import sys

SDK = pathlib.Path(__file__).resolve().parent.parent
TARGET = SDK / "targets" / "mips3-sgi-irix7-native-n32.json"
USER_ABI = SDK / "user-abi" / "src" / "lib.rs"
USER_LINKER = SDK / "linker" / "user.ld"
USER_TOOL = SDK / "tools" / "irix7-user.py"


def read(path):
    try:
        return path.read_text()
    except OSError as error:
        raise SystemExit(f"cannot read {path}: {error}")


def compare_mapping(label, expected, actual, failures):
    if expected == actual:
        print(f"ok: {label}")
        return
    failures.append(label)
    for key in sorted(set(expected) | set(actual)):
        if expected.get(key) != actual.get(key):
            print(
                f"drift: {label}.{key}: sdk={expected.get(key)!r} "
                f"os={actual.get(key)!r}"
            )


def compare_target(os_root, failures):
    os_target = os_root / "targets" / "mips3-sgi-irix7-native-n32.json"
    expected = json.loads(TARGET.read_text())
    actual = json.loads(read(os_target))
    compare_mapping("user target", expected, actual, failures)


def syscall_numbers(text):
    return {
        name: int(value)
        for name, value in re.findall(
            r"pub const (SYS_[A-Z0-9_]+): usize = (\d+);", text
        )
    }


def compare_syscalls(os_root, failures):
    expected = syscall_numbers(USER_ABI.read_text())
    actual = syscall_numbers(read(os_root / "crates" / "irix7-libc" / "src" / "sys.rs"))
    compare_mapping("syscall numbers", expected, actual, failures)


def struct_fields(text, name):
    match = re.search(
        r"struct\s+" + re.escape(name) + r"\s*\{(?P<body>.*?)\n\s*\}",
        text,
        re.S,
    )
    if match is None:
        return None
    fields = []
    for line in match.group("body").splitlines():
        line = line.strip()
        if not line or line.startswith("//") or line.startswith("#"):
            continue
        field = re.match(
            r"(?:pub\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*:\s*(?P<type>[^,]+),",
            line,
        )
        if field:
            field_type = re.sub(r"\s+", " ", field.group("type")).strip()
            fields.append((field.group("name"), field_type))
    return fields


def compare_stat(os_root, failures):
    local = struct_fields(USER_ABI.read_text(), "Stat")
    shim = struct_fields(
        read(os_root / "crates" / "irix7-libc" / "src" / "sys.rs"), "Stat"
    )
    kernel = struct_fields(read(os_root / "kernel" / "src" / "user.rs"), "KernelStat")
    for label, other in (("crates/irix7-libc Stat", shim), ("kernel KernelStat", kernel)):
        if local is None or other is None:
            failures.append(label)
            print(f"drift: {label} could not be parsed")
        elif local != other:
            failures.append(label)
            print(f"drift: {label} fields differ")
            print(f"  sdk:    {local}")
            print(f"  os:     {other}")
        else:
            print(f"ok: {label} fields")


def compare_linker(os_root, failures):
    sdk = read(USER_LINKER)
    os_linker = read(os_root / "user" / "hello" / "linker.ld")
    sdk_base = re.search(r"\.\s*=\s*0x([0-9A-Fa-f]+);", sdk)
    os_base = re.search(r"\.\s*=\s*0x([0-9A-Fa-f]+);", os_linker)
    checks = {
        "linker format": (
            'OUTPUT_FORMAT("elf32-ntradbigmips")' in sdk,
            'OUTPUT_FORMAT("elf32-ntradbigmips")' in os_linker,
        ),
        "linker load address": (
            sdk_base.group(1).lower() if sdk_base else None,
            os_base.group(1).lower() if os_base else None,
        ),
    }
    for label, (expected, actual) in checks.items():
        if expected == actual and expected is not None:
            print(f"ok: {label}")
        else:
            failures.append(label)
            print(f"drift: {label}: sdk={expected!r} os={actual!r}")


def evaluate(expression):
    if not re.fullmatch(r"[0-9_+\-*/() ]+", expression):
        return None
    try:
        return eval(expression, {"__builtins__": None}, {})  # noqa: S307 - checked above
    except (SyntaxError, ZeroDivisionError):
        return None


def compare_loader(os_root, failures):
    kernel = read(os_root / "kernel" / "src" / "user.rs")
    tool = read(USER_TOOL)
    os_size = re.search(r"const ELF_SIZE: usize = ([^;]+);", kernel)
    sdk_size = re.search(r"IMAGE_SIZE = ([0-9_+\-*/() ]+)", tool)
    expected_size = evaluate(os_size.group(1)) if os_size else None
    actual_size = evaluate(sdk_size.group(1)) if sdk_size else None
    if expected_size == actual_size == 256 * 1024:
        print("ok: loader window 256 KiB")
    else:
        failures.append("loader window")
        print(f"drift: loader window: sdk={actual_size!r} os={expected_size!r}")

    match = re.search(r"const TARGET: u8 = (\d+);", kernel)
    if match and match.group(1) == "2":
        print("ok: SCSI target 2")
    else:
        failures.append("SCSI target")
        print("drift: kernel user image target is not 2")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--os",
        default=os.environ.get("IRIX7_OS"),
        help="path to an irix7/os checkout (or set IRIX7_OS)",
    )
    args = parser.parse_args()
    if not args.os:
        print("error: pass --os PATH or set IRIX7_OS", file=sys.stderr)
        return 2
    os_root = pathlib.Path(args.os).resolve()
    if not (os_root / "kernel" / "src" / "user.rs").exists():
        print(f"error: {os_root} does not look like an irix7/os checkout", file=sys.stderr)
        return 2

    failures = []
    compare_target(os_root, failures)
    compare_syscalls(os_root, failures)
    compare_stat(os_root, failures)
    compare_linker(os_root, failures)
    compare_loader(os_root, failures)
    if failures:
        print(f"\n{len(failures)} sync check(s) failed", file=sys.stderr)
        return 1
    print("\nuser target, syscall ABI, stat layout, linker and loader agree")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
