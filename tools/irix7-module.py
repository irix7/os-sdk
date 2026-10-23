#!/usr/bin/env python3
"""Build an IRIX 7 loadable module into its flat image.

Runs cargo with the SDK's MIPS target and linker script, flattens the ELF with
llvm-objcopy, and prints the size, SHA-256 and a ready-to-paste console command.

Example:
    tools/irix7-module.py --package hello-sdk
    tools/irix7-module.py --manifest-path my-module/Cargo.toml \
        --package my-module --linker my-module/linker.ld

The cargo target directory is read from `cargo metadata`, so workspaces and a
separate CARGO_TARGET_DIR are handled.
"""

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys


def run(cmd, cwd, env=None):
    print("+ " + " ".join(str(part) for part in cmd))
    subprocess.run(cmd, cwd=cwd, env=env, check=True)


def find_objcopy():
    sysroot = subprocess.check_output(
        ["rustc", "--print", "sysroot"], text=True
    ).strip()
    host = None
    for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines():
        if line.startswith("host: "):
            host = line[len("host: ") :]
            break
    if host:
        candidate = (
            pathlib.Path(sysroot) / "lib" / "rustlib" / host / "bin" / "llvm-objcopy"
        )
        if candidate.exists():
            return str(candidate)
    found = shutil.which("llvm-objcopy")
    if found:
        return found
    raise SystemExit(
        "llvm-objcopy not found; install the rust-src component or put llvm-objcopy on PATH"
    )


def main():
    sdk = pathlib.Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", default="hello-sdk", help="cargo package to build")
    parser.add_argument(
        "--artifact",
        help="binary name inside the package (default: the package's first bin)",
    )
    parser.add_argument(
        "--manifest-path",
        default=str(sdk / "examples" / "hello" / "Cargo.toml"),
        help="package manifest",
    )
    parser.add_argument("--output", help="flat image path")
    parser.add_argument(
        "--target",
        default=str(sdk / "targets" / "mips3-sgi-irix7-n32.json"),
        help="target JSON",
    )
    parser.add_argument(
        "--linker",
        default=str(sdk / "linker" / "module.ld"),
        help="flat-image linker script",
    )
    parser.add_argument("--debug", action="store_true", help="build the debug profile")
    args = parser.parse_args()

    manifest = pathlib.Path(args.manifest_path).resolve()
    target = pathlib.Path(args.target).resolve()
    linker = pathlib.Path(args.linker).resolve()
    if not manifest.exists():
        raise SystemExit(f"manifest not found: {manifest}")
    if not target.exists():
        raise SystemExit(f"target JSON not found: {target}")
    if not linker.exists():
        raise SystemExit(f"linker script not found: {linker}")

    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
                str(manifest),
            ],
            cwd=manifest.parent,
        )
    )
    package = next(
        (pkg for pkg in metadata["packages"] if pkg["name"] == args.package), None
    )
    if package is None:
        raise SystemExit(f"package {args.package!r} not found in {manifest}")
    bins = [t for t in package["targets"] if "bin" in t["kind"]]
    artifact = args.artifact or (bins[0]["name"] if bins else args.package)
    profile = "debug" if args.debug else "release"
    stem = target.stem
    target_dir = pathlib.Path(metadata["target_directory"])

    env = os.environ.copy()
    var = "CARGO_TARGET_" + stem.upper().replace("-", "_") + "_RUSTFLAGS"
    flags = f"-Clink-arg=-T{linker}"
    if env.get(var):
        flags = env[var] + " " + flags
    env[var] = flags

    cmd = [
        "cargo",
        "build",
        "-Z",
        "build-std=core",
        "-Z",
        "json-target-spec",
        "--target",
        str(target),
        "--manifest-path",
        str(manifest),
        "-p",
        args.package,
    ]
    if not args.debug:
        cmd.append("--release")
    run(cmd, cwd=manifest.parent, env=env)

    elf = target_dir / stem / profile / artifact
    if not elf.exists():
        raise SystemExit(f"build artifact not found: {elf}")
    output = (
        pathlib.Path(args.output).resolve()
        if args.output
        else pathlib.Path.cwd() / f"{artifact}.module"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    run([find_objcopy(), "-O", "binary", str(elf), str(output)], cwd=manifest.parent)

    image = output.read_bytes()
    digest = hashlib.sha256(image).hexdigest()
    print()
    print(f"module: {output}")
    print(f"size:   {len(image)} bytes")
    print(f"sha256: {digest}")
    print(f"load:   module http://<host>:<port>/{output.name} {digest}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        print(f"command failed with status {error.returncode}", file=sys.stderr)
        raise SystemExit(error.returncode)
