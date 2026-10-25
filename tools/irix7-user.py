#!/usr/bin/env python3
"""Build an IRIX 7 static native n32 user image.

The tool owns the user target JSON, the user linker script, ELF validation and
the 256 KiB SCSI target-2 image.  It builds with the repository's pinned Rust
nightly and `rust-src`, checks that the result is a static big-endian n32
executable with no undefined symbols, and pads it to exactly 262144 bytes.

Examples:
    tools/irix7-user.py --package user-hello-sdk
    tools/irix7-user.py --manifest-path examples/user-probe/Cargo.toml \
        --package user-probe-sdk --output /tmp/user-probe.elf

The cargo target directory is read from `cargo metadata`, so workspaces and a
separate CARGO_TARGET_DIR are handled.
"""

import argparse
import hashlib
import json
import os
import pathlib
import struct
import subprocess
import sys

# The loader reads at most 256 KiB from LBA 0 of SCSI target 2.
IMAGE_SIZE = 256 * 1024

# ELF constants used by the validator.
ET_EXEC = 2
EM_MIPS = 8
PT_LOAD = 1
PT_DYNAMIC = 2
PT_INTERP = 3
PT_TLS = 7
PF_X = 0x1
PF_W = 0x2
SHT_SYMTAB = 2
SHT_NOBITS = 8
SHT_DYNSYM = 11
SHF_WRITE = 0x1
SHF_ALLOC = 0x2
SHF_EXECINSTR = 0x4
SHN_UNDEF = 0

EF_MIPS_ABI = 0x0000F000
EF_MIPS_ABI2 = 0x00000020
EF_MIPS_ARCH = 0xF0000000
EF_MIPS_ARCH_3 = 0x20000000
EF_MIPS_PIC = 0x00000002

MAX_U32 = 0xFFFF_FFFF


class ValidationError(Exception):
    """The built file is not a valid IRIX 7 static user image."""


def _u16(data, offset):
    if offset + 2 > len(data):
        raise ValidationError(f"truncated 16-bit field at offset {offset}")
    return struct.unpack_from(">H", data, offset)[0]


def _u32(data, offset):
    if offset + 4 > len(data):
        raise ValidationError(f"truncated 32-bit field at offset {offset}")
    return struct.unpack_from(">I", data, offset)[0]


def _c_string(table, offset):
    if offset >= len(table):
        raise ValidationError(f"string offset {offset} outside table")
    end = table.find(b"\0", offset)
    if end < 0:
        raise ValidationError(f"unterminated string at offset {offset}")
    return table[offset:end].decode("utf-8", "replace")


def parse_elf(data):
    """Parse the ELF32 big-endian structures used by the loader.

    The parser is deliberately strict about the fields the kernel loader reads.
    It does not try to be a general-purpose ELF reader.
    """
    if len(data) < 52:
        raise ValidationError("file is shorter than the ELF32 header")
    if data[:4] != b"\x7fELF":
        raise ValidationError("missing ELF magic")
    if data[4] != 1:
        raise ValidationError("not ELFCLASS32")
    if data[5] != 2:
        raise ValidationError("not big-endian")
    if data[6] != 1:
        raise ValidationError("unsupported ELF identification version")

    header = {
        "type": _u16(data, 16),
        "machine": _u16(data, 18),
        "version": _u32(data, 20),
        "entry": _u32(data, 24),
        "phoff": _u32(data, 28),
        "shoff": _u32(data, 32),
        "flags": _u32(data, 36),
        "ehsize": _u16(data, 40),
        "phentsize": _u16(data, 42),
        "phnum": _u16(data, 44),
        "shentsize": _u16(data, 46),
        "shnum": _u16(data, 48),
        "shstrndx": _u16(data, 50),
    }
    if header["ehsize"] < 52:
        raise ValidationError(f"e_ehsize {header['ehsize']} is smaller than 52")
    if header["type"] != ET_EXEC:
        raise ValidationError(f"e_type {header['type']} is not ET_EXEC")
    if header["machine"] != EM_MIPS:
        raise ValidationError(f"e_machine {header['machine']} is not EM_MIPS")
    if header["version"] != 1:
        raise ValidationError("unsupported ELF version")

    segments = []
    if header["phnum"]:
        if header["phentsize"] < 32:
            raise ValidationError("program-header entry is smaller than 32 bytes")
        end = header["phoff"] + header["phnum"] * header["phentsize"]
        if end > len(data):
            raise ValidationError("program-header table extends past end of file")
        for index in range(header["phnum"]):
            base = header["phoff"] + index * header["phentsize"]
            segment = {
                "type": _u32(data, base),
                "offset": _u32(data, base + 4),
                "vaddr": _u32(data, base + 8),
                "paddr": _u32(data, base + 12),
                "filesz": _u32(data, base + 16),
                "memsz": _u32(data, base + 20),
                "flags": _u32(data, base + 24),
                "align": _u32(data, base + 28),
            }
            if segment["filesz"] > segment["memsz"]:
                raise ValidationError(f"segment {index} has p_filesz > p_memsz")
            if segment["offset"] + segment["filesz"] > len(data):
                raise ValidationError(f"segment {index} extends past end of file")
            if segment["vaddr"] + segment["memsz"] > MAX_U32 + 1:
                raise ValidationError(f"segment {index} overflows the 32-bit address space")
            align = segment["align"]
            if align not in (0, 1) and (align & (align - 1)) != 0:
                raise ValidationError(f"segment {index} has a non-power-of-two p_align")
            if align > 1 and segment["vaddr"] % align != segment["offset"] % align:
                raise ValidationError(f"segment {index} violates p_vaddr % p_align")
            segments.append(segment)

    sections = []
    if header["shnum"]:
        if header["shentsize"] < 40:
            raise ValidationError("section-header entry is smaller than 40 bytes")
        end = header["shoff"] + header["shnum"] * header["shentsize"]
        if end > len(data):
            raise ValidationError("section-header table extends past end of file")
        if header["shstrndx"] >= header["shnum"]:
            raise ValidationError("e_shstrndx is outside the section table")
        headers = []
        for index in range(header["shnum"]):
            base = header["shoff"] + index * header["shentsize"]
            headers.append(
                {
                    "name_offset": _u32(data, base),
                    "type": _u32(data, base + 4),
                    "flags": _u32(data, base + 8),
                    "addr": _u32(data, base + 12),
                    "offset": _u32(data, base + 16),
                    "size": _u32(data, base + 20),
                    "link": _u32(data, base + 24),
                    "info": _u32(data, base + 28),
                    "addralign": _u32(data, base + 32),
                    "entsize": _u32(data, base + 36),
                }
            )
        string_header = headers[header["shstrndx"]]
        if string_header["type"] == SHT_NOBITS:
            raise ValidationError("section-name table is NOBITS")
        if string_header["offset"] + string_header["size"] > len(data):
            raise ValidationError("section-name table extends past end of file")
        string_table = data[
            string_header["offset"] : string_header["offset"] + string_header["size"]
        ]
        for index, section in enumerate(headers):
            section["index"] = index
            section["name"] = _c_string(string_table, section["name_offset"])
            if section["type"] != SHT_NOBITS:
                if section["offset"] + section["size"] > len(data):
                    raise ValidationError(f"section {section['name']} extends past end of file")
            sections.append(section)

    return {
        "header": header,
        "segments": segments,
        "sections": sections,
    }


def _section_map(sections):
    return {section["name"]: section for section in sections}


def _inside_segment(address, size, segment):
    start = segment["vaddr"]
    end = segment["vaddr"] + segment["memsz"]
    return start <= address and address + size <= end


def validate_user_elf(data, image_limit=IMAGE_SIZE):
    """Validate a static n32 user image and return a summary.

    Raises :class:`ValidationError` with a diagnostic when the image does not
    match the loader contract.
    """
    if len(data) > image_limit:
        raise ValidationError(
            f"ELF is {len(data)} bytes, larger than the {image_limit}-byte loader window"
        )

    parsed = parse_elf(data)
    header = parsed["header"]
    segments = parsed["segments"]
    sections = parsed["sections"]

    flags = header["flags"]
    if flags & EF_MIPS_ABI:
        raise ValidationError(f"e_flags 0x{flags:08x} selects an ABI other than n32")
    if not flags & EF_MIPS_ABI2:
        raise ValidationError(f"e_flags 0x{flags:08x} does not set EF_MIPS_ABI2 (n32)")
    if flags & EF_MIPS_ARCH != EF_MIPS_ARCH_3:
        raise ValidationError(f"e_flags 0x{flags:08x} is not MIPS III")
    if flags & EF_MIPS_PIC:
        raise ValidationError(f"e_flags 0x{flags:08x} marks a position-independent image")

    load_segments = [segment for segment in segments if segment["type"] == PT_LOAD]
    if not load_segments:
        raise ValidationError("image has no PT_LOAD segment")
    if not any(segment["memsz"] for segment in load_segments):
        raise ValidationError("every PT_LOAD segment is empty")

    for segment in segments:
        if segment["type"] == PT_DYNAMIC:
            raise ValidationError("image contains PT_DYNAMIC; the loader is static-only")
        if segment["type"] == PT_INTERP:
            raise ValidationError("image contains PT_INTERP; the loader is static-only")
        if segment["type"] == PT_TLS:
            raise ValidationError("image contains PT_TLS; the bring-up runtime has no TLS")

    entry = header["entry"]
    entry_segment = next(
        (
            segment
            for segment in load_segments
            if segment["flags"] & PF_X
            and segment["vaddr"] <= entry < segment["vaddr"] + segment["memsz"]
        ),
        None,
    )
    if entry_segment is None:
        raise ValidationError(f"entry 0x{entry:08x} is not inside an executable PT_LOAD")

    names = _section_map(sections)
    text = names.get(".text")
    if text is None:
        raise ValidationError("image has no .text section")
    if not text["flags"] & SHF_ALLOC or not text["flags"] & SHF_EXECINSTR:
        raise ValidationError(".text is not an allocated executable section")
    if text["flags"] & SHF_WRITE:
        raise ValidationError(".text is writable")
    if not (text["addr"] <= entry < text["addr"] + text["size"]):
        raise ValidationError(f"entry 0x{entry:08x} is not inside .text")
    if ".rodata" not in names:
        raise ValidationError("image has no .rodata section")
    if not names[".rodata"]["flags"] & SHF_ALLOC:
        raise ValidationError(".rodata is not allocated")
    if names[".rodata"]["flags"] & SHF_WRITE:
        raise ValidationError(".rodata is writable")

    for section in sections:
        if not section["flags"] & SHF_ALLOC or section["size"] == 0:
            continue
        if not any(_inside_segment(section["addr"], section["size"], segment) for segment in load_segments):
            raise ValidationError(
                f"allocated section {section['name']} is not covered by a PT_LOAD segment"
            )

    symtab = next((section for section in sections if section["type"] == SHT_SYMTAB), None)
    if symtab is None:
        raise ValidationError("image has no .symtab; cannot check for undefined symbols")
    if symtab["entsize"] and symtab["entsize"] < 16:
        raise ValidationError("symbol-table entry is smaller than 16 bytes")
    entry_size = symtab["entsize"] or 16
    if symtab["link"] >= len(sections):
        raise ValidationError("symbol-table string table link is invalid")
    linked = sections[symtab["link"]]
    if linked["offset"] + linked["size"] > len(data):
        raise ValidationError("symbol string table extends past end of file")
    strings = data[linked["offset"] : linked["offset"] + linked["size"]]
    undefined = []
    count = symtab["size"] // entry_size
    for index in range(count):
        base = symtab["offset"] + index * entry_size
        name_offset = _u32(data, base)
        shndx = _u16(data, base + 14)
        if shndx == SHN_UNDEF and index != 0:
            name = _c_string(strings, name_offset) if name_offset else "<unnamed>"
            undefined.append(name)
    if undefined:
        raise ValidationError("undefined symbols: " + ", ".join(sorted(set(undefined))))
    if any(section["type"] == SHT_DYNSYM for section in sections):
        raise ValidationError("image contains a dynamic symbol table")

    return {
        "entry": entry,
        "flags": flags,
        "elf_size": len(data),
        "segments": [
            {
                "type": segment["type"],
                "vaddr": segment["vaddr"],
                "filesz": segment["filesz"],
                "memsz": segment["memsz"],
                "flags": segment["flags"],
            }
            for segment in segments
            if segment["type"] == PT_LOAD
        ],
        "sections": [
            {
                "name": section["name"],
                "addr": section["addr"],
                "size": section["size"],
                "flags": section["flags"],
            }
            for section in sections
            if section["flags"] & SHF_ALLOC
        ],
    }


def build_image(elf, size=IMAGE_SIZE):
    """Pad a validated ELF to the exact raw SCSI image size."""
    if len(elf) > size:
        raise ValidationError(
            f"ELF is {len(elf)} bytes, larger than the {size}-byte image"
        )
    return elf + b"\0" * (size - len(elf))


def _run(command, cwd, env=None):
    print("+ " + " ".join(str(part) for part in command), file=sys.stderr)
    subprocess.run(command, cwd=cwd, env=env, check=True)


def _build(args, sdk):
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
    bins = [item for item in package["targets"] if "bin" in item["kind"]]
    artifact = args.artifact or (bins[0]["name"] if bins else args.package)
    profile = "debug" if args.debug else "release"
    stem = target.stem
    target_dir = pathlib.Path(metadata["target_directory"])

    env = os.environ.copy()
    variable = "CARGO_TARGET_" + stem.upper().replace("-", "_") + "_RUSTFLAGS"
    flags = (
        f"-Clink-arg=-T{linker} "
        "-Clink-arg=--undefined=_start"
    )
    if env.get(variable):
        flags = env[variable] + " " + flags
    env[variable] = flags

    command = [
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
        command.append("--release")
    _run(command, cwd=manifest.parent, env=env)

    elf_path = target_dir / stem / profile / artifact
    if not elf_path.exists():
        raise SystemExit(f"build artifact not found: {elf_path}")
    return elf_path, artifact


def _write_outputs(args, elf_path, artifact):
    data = elf_path.read_bytes()
    summary = validate_user_elf(data, args.image_size)
    output = (
        pathlib.Path(args.output).resolve()
        if args.output
        else pathlib.Path.cwd() / f"{artifact}.elf"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(data)

    image = None
    if not args.no_image:
        image = (
            pathlib.Path(args.image).resolve()
            if args.image
            else output.with_suffix(".raw")
        )
        if image == output:
            raise SystemExit("--image must differ from --output")
        image.parent.mkdir(parents=True, exist_ok=True)
        image.write_bytes(build_image(data, args.image_size))
    return output, image, summary


def main():
    sdk = pathlib.Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--package", default="user-hello-sdk", help="cargo package to build"
    )
    parser.add_argument(
        "--artifact",
        help="binary name inside the package (default: the package's first bin)",
    )
    parser.add_argument(
        "--manifest-path",
        default=str(sdk / "examples" / "user-hello" / "Cargo.toml"),
        help="package manifest",
    )
    parser.add_argument("--output", help="ELF output path")
    parser.add_argument("--image", help="raw target-2 image path")
    parser.add_argument(
        "--image-size",
        type=int,
        default=IMAGE_SIZE,
        help=f"raw image size in bytes (default: {IMAGE_SIZE})",
    )
    parser.add_argument(
        "--no-image", action="store_true", help="write only the ELF"
    )
    parser.add_argument(
        "--target",
        default=str(sdk / "targets" / "mips3-sgi-irix7-native-n32.json"),
        help="user target JSON",
    )
    parser.add_argument(
        "--linker",
        default=str(sdk / "linker" / "user.ld"),
        help="user linker script",
    )
    parser.add_argument("--debug", action="store_true", help="build the debug profile")
    parser.add_argument(
        "--json", action="store_true", help="print machine-readable output on stdout"
    )
    args = parser.parse_args()
    if args.image_size <= 0:
        raise SystemExit("--image-size must be positive")
    if args.image_size % 512 != 0:
        raise SystemExit("--image-size must be a multiple of the 512-byte SCSI sector")

    elf_path, artifact = _build(args, sdk)
    try:
        output, image, summary = _write_outputs(args, elf_path, artifact)
    except ValidationError as error:
        print(f"validation failed: {error}", file=sys.stderr)
        return 1

    image_size = image.stat().st_size if image else 0
    digest = hashlib.sha256(image.read_bytes()).hexdigest() if image else ""
    result = {
        "elf": str(output),
        "image": str(image) if image else None,
        "elf_size": summary["elf_size"],
        "image_size": image_size,
        "sha256": digest,
        "entry": f"0x{summary['entry']:08x}",
        "sections": [section["name"] for section in summary["sections"]],
    }
    if args.json:
        print(json.dumps(result))
    else:
        print()
        print(f"elf:    {output}")
        if image:
            print(f"image:  {image}")
            print(f"size:   {summary['elf_size']} bytes (ELF)")
            print(f"image:  {image_size} bytes (raw SCSI target 2)")
            print(f"entry:  0x{summary['entry']:08x}")
            print(f"sha256: {digest}")
        else:
            print(f"size:   {summary['elf_size']} bytes (ELF)")
            print(f"entry:  0x{summary['entry']:08x}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        print(f"command failed with status {error.returncode}", file=sys.stderr)
        raise SystemExit(error.returncode)
