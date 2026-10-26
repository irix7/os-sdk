#!/usr/bin/env python3
"""Build an IRIX 7 loadable module into its flat image.

Runs cargo with the SDK's MIPS target and linker script, flattens the ELF with
llvm-objcopy, and prints the size, SHA-256 and a ready-to-paste console command.

A module may declare a manifest. Pass ``--manifest-json PATH`` or the explicit
``--name``/``--version``/``--abi-major``/``--abi-minor`` flags, with optional
``--depends`` and ``--export``. The tool appends the version 2 ``I7MM`` record
and ``I7MF`` footer after the flattened image and hashes the complete image, so
the printed digest is the one the kernel checks. Without a manifest option the
output is exactly the flattened payload, as before.

Example:
    tools/irix7-module.py --package hello-sdk
    tools/irix7-module.py --manifest-path my-module/Cargo.toml \\
        --package my-module --linker my-module/linker.ld
    tools/irix7-module.py --package my-module \\
        --manifest-json my-module/manifest.json

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

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import irix7_manifest

MANIFEST_FLAGS = (
    "name",
    "version",
    "abi_major",
    "abi_minor",
    "entry_offset",
    "depends",
    "export",
)


def run(cmd, cwd, env=None):
    print("+ " + " ".join(str(part) for part in cmd), file=sys.stderr)
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
        "llvm-objcopy not found; install the llvm-tools component or put llvm-objcopy on PATH"
    )


def _provided(value):
    return value is not None and value != []


def _flag_name(flag):
    return "--" + flag.replace("_", "-")


def build_manifest(args, sdk):
    """Build and validate the optional manifest from the command line.

    Returns ``None`` when no manifest option was given, which keeps the flat
    image identical to the pre-manifest tool.
    """
    explicit = [flag for flag in MANIFEST_FLAGS if _provided(getattr(args, flag))]
    if args.manifest_json:
        if explicit:
            raise irix7_manifest.ManifestError(
                "--manifest-json cannot be combined with "
                + ", ".join(_flag_name(flag) for flag in explicit)
            )
        manifest = irix7_manifest.load_manifest_json(args.manifest_json)
    elif explicit:
        missing = [
            flag
            for flag in ("name", "version", "abi_major", "abi_minor")
            if getattr(args, flag) is None
        ]
        if missing:
            raise irix7_manifest.ManifestError(
                "manifest is missing " + ", ".join(_flag_name(flag) for flag in missing)
            )
        manifest = irix7_manifest.Manifest(
            name=args.name,
            version=args.version,
            abi_major=args.abi_major,
            abi_minor=args.abi_minor,
            entry_offset=args.entry_offset or 0,
            dependencies=tuple(
                irix7_manifest.parse_dependency_spec(spec)
                for spec in args.depends or []
            ),
            exports=tuple(
                irix7_manifest.parse_export_spec(spec) for spec in args.export or []
            ),
        )
    else:
        return None
    irix7_manifest.validate_manifest(manifest, irix7_manifest.sdk_module_abi(sdk))
    return manifest


def finish_image(image, manifest):
    """Wrap the flat image and hash the bytes the kernel will verify.

    Returns ``(image_with_footer, sha256 hex, record length)``. With no
    manifest the image is unchanged and the digest is the plain flat image.
    """
    if manifest is None:
        return image, hashlib.sha256(image).hexdigest(), 0
    wrapped = irix7_manifest.wrap_image(image, manifest)
    record_len = len(wrapped) - len(image) - irix7_manifest.FOOTER_LEN
    return wrapped, hashlib.sha256(wrapped).hexdigest(), record_len


def main():
    sdk = pathlib.Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
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
    parser.add_argument(
        "--json", action="store_true", help="print {module,size,sha256} on stdout"
    )
    manifest_group = parser.add_argument_group(
        "module manifest",
        "Append a version 2 I7MM record and I7MF footer. Without any of these "
        "options the output is the plain flattened image.",
    )
    manifest_group.add_argument(
        "--manifest-json",
        metavar="PATH",
        help="manifest JSON document (see docs/module-manifest.md)",
    )
    manifest_group.add_argument("--name", help="module name for an inline manifest")
    manifest_group.add_argument(
        "--version", help="exact module version for an inline manifest"
    )
    manifest_group.add_argument(
        "--abi-major",
        type=int,
        help="module service table major generation (1)",
    )
    manifest_group.add_argument(
        "--abi-minor",
        type=int,
        help="module service table ABI revision (abi/src/lib.rs ABI_VERSION)",
    )
    manifest_group.add_argument(
        "--entry-offset",
        type=int,
        help="image-relative manager entry offset (default 0 for _start)",
    )
    manifest_group.add_argument(
        "--depends",
        action="append",
        metavar="NAME@VERSION:SHA256",
        help="exact dependency with its image digest; may be repeated",
    )
    manifest_group.add_argument(
        "--export",
        action="append",
        metavar="INTERFACE@VERSION[:OFFSET]",
        help="exported interface and optional table offset; may be repeated",
    )
    args = parser.parse_args()

    manifest = build_manifest(args, sdk)
    cargo_manifest = pathlib.Path(args.manifest_path).resolve()
    target = pathlib.Path(args.target).resolve()
    linker = pathlib.Path(args.linker).resolve()
    if not cargo_manifest.exists():
        raise SystemExit(f"manifest not found: {cargo_manifest}")
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
                str(cargo_manifest),
            ],
            cwd=cargo_manifest.parent,
        )
    )
    package = next(
        (pkg for pkg in metadata["packages"] if pkg["name"] == args.package), None
    )
    if package is None:
        raise SystemExit(f"package {args.package!r} not found in {cargo_manifest}")
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
        str(cargo_manifest),
        "-p",
        args.package,
    ]
    if not args.debug:
        cmd.append("--release")
    run(cmd, cwd=cargo_manifest.parent, env=env)

    elf = target_dir / stem / profile / artifact
    if not elf.exists():
        raise SystemExit(f"build artifact not found: {elf}")
    output = (
        pathlib.Path(args.output).resolve()
        if args.output
        else pathlib.Path.cwd() / f"{artifact}.module"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    run([find_objcopy(), "-O", "binary", str(elf), str(output)], cwd=cargo_manifest.parent)

    image = output.read_bytes()
    if manifest is not None:
        irix7_manifest.validate_image_offsets(manifest, len(image))
    wrapped, digest, record_len = finish_image(image, manifest)
    if manifest is not None:
        output.write_bytes(wrapped)

    if args.json:
        result = {"module": str(output), "size": len(wrapped), "sha256": digest}
        if manifest is not None:
            result["image_size"] = len(image)
            result["manifest_size"] = record_len
            result["footer_size"] = irix7_manifest.FOOTER_LEN
            result["manifest"] = irix7_manifest.manifest_to_json(manifest)
        print(json.dumps(result))
    else:
        print()
        print(f"module: {output}")
        print(f"size:   {len(wrapped)} bytes")
        print(f"sha256: {digest}")
        if manifest is not None:
            dependencies = len(manifest.dependencies)
            exports = len(manifest.exports)
            print(f"image:  {len(image)} bytes (flattened image before the manifest)")
            print(
                f"manifest: {manifest.name}@{manifest.version} "
                f"abi {manifest.abi_major}.{manifest.abi_minor}, "
                f"entry {manifest.entry_offset}, "
                f"{dependencies} "
                f"{'dependency' if dependencies == 1 else 'dependencies'}, "
                f"{exports} {'export' if exports == 1 else 'exports'} "
                f"(record {record_len} bytes, footer {irix7_manifest.FOOTER_LEN} bytes)"
            )
        print(f"load:   module http://<host>:<port>/{output.name} {digest}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except irix7_manifest.ManifestError as error:
        print(f"manifest: {error}", file=sys.stderr)
        raise SystemExit(2)
    except subprocess.CalledProcessError as error:
        print(f"command failed with status {error.returncode}", file=sys.stderr)
        raise SystemExit(error.returncode)
