#!/usr/bin/env python3
"""I7MM manifest records (version 2, legacy version 1) and I7MF footers.

This is the build-time SDK implementation of the module manifest format
defined by ``crates/module-resolver`` in irix7/os (``docs/spec/modules.md``
sections 8.1 and 8.2). Version 1 bytes are little-endian and identical to the
OS Rust encoder; ``testdata/manifest-v1.txt`` and ``manifest-minimal.txt`` are
the OS-produced compatibility vectors. Version 2 adds the manager entry offset
and per-export table offsets and is emitted by this encoder.

A module image may carry a manifest after its flattened payload::

    image bytes | I7MM record | I7MF footer

The version 2 record is::

    I7MM | u16 format=2 | u16 abi major | u16 abi minor
         | u32 entry offset
         | u16 name length | UTF-8 name
         | u16 version length | UTF-8 version
         | u16 dependency count
         | (u16 name length, name, u16 version length, version, 32-byte SHA-256)*
         | u16 export count
         | (u16 interface length, interface, u16 export version, u32 table offset)*

Version 1 is the same record without the entry offset and table offsets; both
decode as zero.

The footer is 16 bytes::

    I7MF | u16 version=1 | u16 reserved=0 | u32 manifest offset | u32 manifest length

There is no CRC. The SHA-256 of the complete image, including the record and
the footer, is the integrity boundary. A standalone image without a footer
remains valid.

``validate_manifest`` adds the SDK's build-time rules (symbol charset,
duplicate and conflicting dependencies, duplicate exports, offset alignment
and the service table ABI) that the wire format intentionally does not enforce.
"""

from __future__ import annotations

import json
import pathlib
import re
import struct
from dataclasses import dataclass
from typing import Any

MAGIC = b"I7MM"
FORMAT_VERSION = 2
LEGACY_FORMAT_VERSION = 1
MAX_RECORD = 64 * 1024

FOOTER_MAGIC = b"I7MF"
FOOTER_VERSION = 1
FOOTER_LEN = 16

DIGEST_LEN = 32

#: Entry and export table offsets must be aligned to this many bytes.
MODULE_OFFSET_ALIGN = 4

#: The module service table's breaking generation. The table is append-only,
#: so every additive service bump increments the minor instead.
SERVICE_ABI_MAJOR = 1

#: Names, versions and interfaces use this character set. The separators used
#: by ``--depends`` (``@`` and ``:``) are deliberately excluded.
SYMBOL_RE = re.compile(r"[A-Za-z0-9._+\-]+")
DIGEST_RE = re.compile(r"[0-9a-fA-F]{64}")
ABI_VERSION_RE = re.compile(r"pub const ABI_VERSION: u32 = (\d+);")

MANIFEST_KEYS = frozenset(
    {
        "format",
        "name",
        "version",
        "abi_major",
        "abi_minor",
        "entry_offset",
        "depends",
        "exports",
    }
)
DEPENDENCY_KEYS = frozenset({"name", "version", "sha256"})
EXPORT_KEYS = frozenset({"interface", "version", "offset"})


class ManifestError(ValueError):
    """A manifest, record or footer is malformed."""


@dataclass(frozen=True)
class Dependency:
    """One exact-version dependency and its expected image digest."""

    name: str
    version: str
    sha256: bytes

    @property
    def key(self) -> tuple[str, str]:
        return (self.name, self.version)


@dataclass(frozen=True)
class Export:
    """A versioned interface and its image-relative function-table offset."""

    interface: str
    version: int
    offset: int = 0


@dataclass(frozen=True)
class Manifest:
    """A module manifest without the image bytes."""

    name: str
    version: str
    abi_major: int
    abi_minor: int
    dependencies: tuple[Dependency, ...] = ()
    exports: tuple[Export, ...] = ()
    entry_offset: int = 0


def _u16(value: Any, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ManifestError(f"{field} must be an integer")
    if not 0 <= value <= 0xFFFF:
        raise ManifestError(f"{field} {value} is outside 0..65535")
    return value


def _u32(value: Any, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ManifestError(f"{field} must be an integer")
    if not 0 <= value <= 0xFFFFFFFF:
        raise ManifestError(f"{field} {value} is outside 0..4294967295")
    return value


def _offset(value: Any, field: str) -> int:
    value = _u32(value, field)
    if value % MODULE_OFFSET_ALIGN != 0:
        raise ManifestError(
            f"{field} {value} is not aligned to {MODULE_OFFSET_ALIGN} bytes"
        )
    return value


def _string(value: Any, field: str) -> str:
    if not isinstance(value, str):
        raise ManifestError(f"{field} must be a string")
    if not value:
        raise ManifestError(f"{field} must not be empty")
    if len(value.encode("utf-8")) > 0xFFFF:
        raise ManifestError(f"{field} exceeds 65535 bytes")
    return value


def _symbol(value: Any, field: str) -> str:
    value = _string(value, field)
    if SYMBOL_RE.fullmatch(value) is None:
        raise ManifestError(
            f"{field} {value!r} has unsupported characters; "
            "use A-Z a-z 0-9 . _ + -"
        )
    return value


def parse_digest(text: Any) -> bytes:
    """Parse a 64-digit hexadecimal SHA-256 digest."""
    if not isinstance(text, str) or DIGEST_RE.fullmatch(text) is None:
        raise ManifestError(f"sha256 {text!r} must be 64 hexadecimal digits")
    return bytes.fromhex(text)


def parse_dependency_spec(spec: str) -> Dependency:
    """Parse ``name@version:sha256``."""
    name, at, rest = spec.partition("@")
    if not at or not name or not rest:
        raise ManifestError(f"dependency {spec!r} must be name@version:sha256")
    version, colon, digest = rest.rpartition(":")
    if not colon or not version or not digest:
        raise ManifestError(f"dependency {spec!r} must be name@version:sha256")
    return Dependency(name=name, version=version, sha256=parse_digest(digest))


def parse_export_spec(spec: str) -> Export:
    """Parse ``interface@version`` or ``interface@version:offset``."""
    interface, at, rest = spec.partition("@")
    if not at or not interface or not rest:
        raise ManifestError(f"export {spec!r} must be interface@version[:offset]")
    version, colon, raw_offset = rest.partition(":")
    if re.fullmatch(r"[0-9]+", version) is None:
        raise ManifestError(f"export {spec!r} has a non-numeric version")
    offset = 0
    if colon:
        if re.fullmatch(r"[0-9]+", raw_offset) is None:
            raise ManifestError(f"export {spec!r} has a non-numeric offset")
        offset = _offset(int(raw_offset), f"export {interface!r} offset")
    return Export(
        interface=interface,
        version=_u16(int(version), f"export {interface!r} version"),
        offset=offset,
    )


def _put_string(out: bytearray, value: str) -> None:
    encoded = value.encode("utf-8")
    if len(encoded) > 0xFFFF:
        raise ManifestError("manifest string exceeds 65535 bytes")
    out += struct.pack("<H", len(encoded))
    out += encoded


def _put_count(out: bytearray, count: int, field: str) -> None:
    if count > 0xFFFF:
        raise ManifestError(f"manifest {field} count {count} exceeds 65535")
    out += struct.pack("<H", count)


def encode_manifest(manifest: Manifest) -> bytes:
    """Encode a version 2 ``I7MM`` record.

    This performs only the bounds the wire format enforces: ``u16`` lengths,
    ``u16`` counts and the 64 KiB record limit. Call :func:`validate_manifest`
    for the SDK's build-time rules.
    """
    out = bytearray(MAGIC)
    out += struct.pack("<H", FORMAT_VERSION)
    out += struct.pack("<H", _u16(manifest.abi_major, "abi_major"))
    out += struct.pack("<H", _u16(manifest.abi_minor, "abi_minor"))
    out += struct.pack("<I", _u32(manifest.entry_offset, "entry_offset"))
    _put_string(out, _string(manifest.name, "name"))
    _put_string(out, _string(manifest.version, "version"))
    _put_count(out, len(manifest.dependencies), "dependency")
    for dependency in manifest.dependencies:
        _put_string(out, _string(dependency.name, "dependency name"))
        _put_string(out, _string(dependency.version, "dependency version"))
        if len(dependency.sha256) != DIGEST_LEN:
            raise ManifestError(
                f"dependency {dependency.name}@{dependency.version} digest must be "
                f"{DIGEST_LEN} bytes"
            )
        out += dependency.sha256
    _put_count(out, len(manifest.exports), "export")
    for export in manifest.exports:
        _put_string(out, _string(export.interface, "export interface"))
        out += struct.pack("<H", _u16(export.version, f"export {export.interface!r} version"))
        out += struct.pack("<I", _u32(export.offset, f"export {export.interface!r} offset"))
    if len(out) > MAX_RECORD:
        raise ManifestError(
            f"manifest record is {len(out)} bytes; the limit is {MAX_RECORD}"
        )
    return bytes(out)


class _Reader:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.pos = 0

    def remaining(self) -> int:
        return len(self.data) - self.pos

    def take(self, length: int) -> bytes:
        end = self.pos + length
        if end > len(self.data):
            raise ManifestError("manifest record is truncated")
        out = self.data[self.pos : end]
        self.pos = end
        return out

    def u16(self) -> int:
        return struct.unpack("<H", self.take(2))[0]

    def u32(self) -> int:
        return struct.unpack("<I", self.take(4))[0]

    def string(self) -> str:
        raw = self.take(self.u16())
        try:
            return raw.decode("utf-8")
        except UnicodeDecodeError as error:
            raise ManifestError("manifest string is not UTF-8") from error


def _read_dependencies(reader: _Reader) -> list[Dependency]:
    dependency_count = reader.u16()
    # Every dependency is at least two u16 lengths plus a 32-byte digest.
    # Reject an impossible count before building a list for it.
    if dependency_count > reader.remaining() // 36:
        raise ManifestError("manifest record is truncated")
    dependencies = []
    for _ in range(dependency_count):
        name = reader.string()
        version = reader.string()
        dependencies.append(Dependency(name, version, reader.take(DIGEST_LEN)))
    return dependencies


def _decode_v1(reader: _Reader) -> Manifest:
    abi_major = reader.u16()
    abi_minor = reader.u16()
    name = reader.string()
    module_version = reader.string()
    dependencies = _read_dependencies(reader)
    export_count = reader.u16()
    if export_count > reader.remaining() // 4:
        raise ManifestError("manifest record is truncated")
    exports = []
    for _ in range(export_count):
        interface = reader.string()
        exports.append(Export(interface, reader.u16()))
    if reader.remaining() != 0:
        raise ManifestError("manifest record has trailing bytes")
    return Manifest(
        name=name,
        version=module_version,
        abi_major=abi_major,
        abi_minor=abi_minor,
        dependencies=tuple(dependencies),
        exports=tuple(exports),
    )


def _decode_v2(reader: _Reader) -> Manifest:
    abi_major = reader.u16()
    abi_minor = reader.u16()
    entry_offset = reader.u32()
    name = reader.string()
    module_version = reader.string()
    dependencies = _read_dependencies(reader)
    export_count = reader.u16()
    # Every export is at least two u16 lengths plus a u32 offset.
    if export_count > reader.remaining() // 8:
        raise ManifestError("manifest record is truncated")
    exports = []
    for _ in range(export_count):
        interface = reader.string()
        version = reader.u16()
        offset = reader.u32()
        exports.append(Export(interface, version, offset))
    if reader.remaining() != 0:
        raise ManifestError("manifest record has trailing bytes")
    return Manifest(
        name=name,
        version=module_version,
        abi_major=abi_major,
        abi_minor=abi_minor,
        dependencies=tuple(dependencies),
        exports=tuple(exports),
        entry_offset=entry_offset,
    )


def decode_manifest(record: bytes) -> Manifest:
    """Decode a version 2 record or a legacy version 1 record.

    A version 1 record decodes with ``entry_offset == 0`` and every export
    offset equal to zero.
    """
    if len(record) > MAX_RECORD:
        raise ManifestError(f"manifest record is {len(record)} bytes; the limit is {MAX_RECORD}")
    reader = _Reader(record)
    if reader.remaining() < len(MAGIC):
        raise ManifestError("manifest record is too short")
    if reader.take(len(MAGIC)) != MAGIC:
        raise ManifestError("manifest record has bad magic")
    version = reader.u16()
    if version == LEGACY_FORMAT_VERSION:
        return _decode_v1(reader)
    if version == FORMAT_VERSION:
        return _decode_v2(reader)
    raise ManifestError(f"unsupported manifest record version {version}")


def wrap_image(image: bytes, manifest: Manifest) -> bytes:
    """Append a manifest record and an ``I7MF`` footer to ``image``."""
    return append_record(image, encode_manifest(manifest))


def append_record(image: bytes, record: bytes) -> bytes:
    """Append an already-encoded record and footer to ``image``."""
    if len(image) > 0xFFFFFFFF:
        raise ManifestError("image exceeds the u32 footer offset")
    if len(record) > 0xFFFFFFFF:
        raise ManifestError("manifest record exceeds the u32 footer length")
    footer = FOOTER_MAGIC + struct.pack(
        "<HHII", FOOTER_VERSION, 0, len(image), len(record)
    )
    return image + record + footer


def parse_wrapped(image: bytes) -> tuple[Manifest, bytes] | None:
    """Parse a footer and its manifest, if present.

    Returns ``None`` for a standalone image without the footer magic. A footer
    that is present but malformed raises :class:`ManifestError`; it is never
    treated as an older standalone image.
    """
    if len(image) < FOOTER_LEN:
        return None
    footer_start = len(image) - FOOTER_LEN
    footer = image[footer_start:]
    if footer[: len(FOOTER_MAGIC)] != FOOTER_MAGIC:
        return None
    version, reserved, offset, length = struct.unpack("<HHII", footer[len(FOOTER_MAGIC) :])
    if version != FOOTER_VERSION:
        raise ManifestError(f"unsupported manifest footer version {version}")
    if reserved != 0:
        raise ManifestError("manifest footer reserved field is non-zero")
    if length > MAX_RECORD:
        raise ManifestError(
            f"manifest footer record is {length} bytes; the limit is {MAX_RECORD}"
        )
    if offset > footer_start or offset + length != footer_start:
        raise ManifestError("manifest footer range is out of bounds")
    manifest = decode_manifest(image[offset : offset + length])
    return manifest, image[:offset]


def validate_manifest(manifest: Manifest, sdk_abi: tuple[int, int]) -> None:
    """Apply the SDK's build-time rules.

    ``sdk_abi`` is the module service table ABI this SDK provides, normally
    from :func:`sdk_module_abi`. A module may declare an older minor but not a
    different major or a newer minor.
    """
    _symbol(manifest.name, "manifest name")
    _symbol(manifest.version, "manifest version")
    abi_major = _u16(manifest.abi_major, "abi_major")
    abi_minor = _u16(manifest.abi_minor, "abi_minor")
    sdk_major, sdk_minor = sdk_abi
    if abi_major != sdk_major:
        raise ManifestError(
            f"unsupported ABI major {abi_major}; the SDK module ABI is "
            f"{sdk_major}.{sdk_minor}"
        )
    if abi_minor > sdk_minor:
        raise ManifestError(
            f"ABI {abi_major}.{abi_minor} is newer than the SDK module ABI "
            f"{sdk_major}.{sdk_minor}"
        )
    _offset(manifest.entry_offset, "entry_offset")

    seen: dict[str, str] = {}
    exact: set[tuple[str, str]] = set()
    for dependency in manifest.dependencies:
        _symbol(dependency.name, "dependency name")
        _symbol(dependency.version, "dependency version")
        if len(dependency.sha256) != DIGEST_LEN:
            raise ManifestError(
                f"dependency {dependency.name}@{dependency.version} digest must be "
                f"{DIGEST_LEN} bytes"
            )
        if dependency.key in exact:
            raise ManifestError(
                f"duplicate dependency {dependency.name}@{dependency.version}"
            )
        exact.add(dependency.key)
        previous = seen.setdefault(dependency.name, dependency.version)
        if previous != dependency.version:
            raise ManifestError(
                f"conflicting dependency versions for {dependency.name}: "
                f"{previous} and {dependency.version}"
            )

    seen_exports: set[tuple[str, int]] = set()
    for export in manifest.exports:
        _symbol(export.interface, "export interface")
        version = _u16(export.version, f"export {export.interface!r} version")
        _offset(export.offset, f"export {export.interface!r} offset")
        key = (export.interface, version)
        if key in seen_exports:
            raise ManifestError(
                f"duplicate export {export.interface}@{export.version}"
            )
        seen_exports.add(key)

    # Enforces the u16 lengths and the 64 KiB record bound.
    encode_manifest(manifest)


def validate_image_offsets(manifest: Manifest, image_size: int) -> None:
    """Check entry and export offsets against the flattened image size.

    Offset zero means "legacy entry" or "no export table" and is always
    accepted. A non-zero offset must leave at least one 32-bit cell inside the
    image; offsets beyond the manifest/footer bytes are rejected.
    """
    if manifest.entry_offset:
        _offset(manifest.entry_offset, "entry_offset")
        if manifest.entry_offset + 4 > image_size:
            raise ManifestError(
                f"entry_offset {manifest.entry_offset} does not fit the "
                f"{image_size}-byte image"
            )
    for export in manifest.exports:
        if export.offset:
            _offset(export.offset, f"export {export.interface!r} offset")
            if export.offset + 4 > image_size:
                raise ManifestError(
                    f"export {export.interface}@{export.version} offset "
                    f"{export.offset} does not fit the {image_size}-byte image"
                )


def manifest_to_json(manifest: Manifest) -> dict[str, Any]:
    """Return the JSON shape accepted by :func:`manifest_from_json`."""
    return {
        "format": FORMAT_VERSION,
        "name": manifest.name,
        "version": manifest.version,
        "abi_major": manifest.abi_major,
        "abi_minor": manifest.abi_minor,
        "entry_offset": manifest.entry_offset,
        "depends": [
            {
                "name": dependency.name,
                "version": dependency.version,
                "sha256": dependency.sha256.hex(),
            }
            for dependency in manifest.dependencies
        ],
        "exports": [
            {
                "interface": export.interface,
                "version": export.version,
                "offset": export.offset,
            }
            for export in manifest.exports
        ],
    }


def _reject_unknown(item: dict[str, Any], allowed: frozenset[str], where: str) -> None:
    unknown = sorted(set(item) - allowed)
    if unknown:
        raise ManifestError(f"{where} has unknown key(s): {', '.join(unknown)}")


def _require(item: dict[str, Any], key: str, where: str) -> Any:
    if key not in item:
        raise ManifestError(f"{where} is missing {key!r}")
    return item[key]


def manifest_from_json(data: Any) -> Manifest:
    """Parse the SDK manifest JSON document (version 2)."""
    if not isinstance(data, dict):
        raise ManifestError("manifest JSON must be an object")
    _reject_unknown(data, MANIFEST_KEYS, "manifest JSON")
    document_format = data.get("format", FORMAT_VERSION)
    if isinstance(document_format, bool) or not isinstance(document_format, int):
        raise ManifestError(f"manifest format {document_format!r} must be an integer")
    if document_format == LEGACY_FORMAT_VERSION:
        raise ManifestError(
            "manifest format 1 is legacy and decode-only; omit format or use 2"
        )
    if document_format != FORMAT_VERSION:
        raise ManifestError(
            f"unsupported manifest format {document_format!r}; "
            f"only {FORMAT_VERSION} is implemented"
        )
    name = _string(_require(data, "name", "manifest JSON"), "manifest name")
    version = _string(
        _require(data, "version", "manifest JSON"), "manifest version"
    )
    abi_major = _u16(_require(data, "abi_major", "manifest JSON"), "abi_major")
    abi_minor = _u16(_require(data, "abi_minor", "manifest JSON"), "abi_minor")
    entry_offset = _offset(data.get("entry_offset", 0), "entry_offset")

    raw_dependencies = data.get("depends", [])
    if not isinstance(raw_dependencies, list):
        raise ManifestError("manifest depends must be a list")
    dependencies = []
    for index, item in enumerate(raw_dependencies):
        where = f"depends[{index}]"
        if not isinstance(item, dict):
            raise ManifestError(f"{where} must be an object")
        _reject_unknown(item, DEPENDENCY_KEYS, where)
        dependencies.append(
            Dependency(
                name=_string(_require(item, "name", where), f"{where} name"),
                version=_string(
                    _require(item, "version", where), f"{where} version"
                ),
                sha256=parse_digest(_require(item, "sha256", where)),
            )
        )

    raw_exports = data.get("exports", [])
    if not isinstance(raw_exports, list):
        raise ManifestError("manifest exports must be a list")
    exports = []
    for index, item in enumerate(raw_exports):
        where = f"exports[{index}]"
        if not isinstance(item, dict):
            raise ManifestError(f"{where} must be an object")
        _reject_unknown(item, EXPORT_KEYS, where)
        exports.append(
            Export(
                interface=_string(
                    _require(item, "interface", where), f"{where} interface"
                ),
                version=_u16(_require(item, "version", where), f"{where} version"),
                offset=_offset(item.get("offset", 0), f"{where} offset"),
            )
        )

    return Manifest(
        name=name,
        version=version,
        abi_major=abi_major,
        abi_minor=abi_minor,
        dependencies=tuple(dependencies),
        exports=tuple(exports),
        entry_offset=entry_offset,
    )


def load_manifest_json(path: pathlib.Path | str) -> Manifest:
    """Read and parse a manifest JSON file."""
    path = pathlib.Path(path)
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ManifestError(f"cannot read manifest {path}: {error}") from error
    try:
        data = json.loads(text)
    except json.JSONDecodeError as error:
        raise ManifestError(f"{path}: {error}") from error
    return manifest_from_json(data)


def sdk_module_abi(sdk_root: pathlib.Path | str) -> tuple[int, int]:
    """Read the module service table ABI from the SDK's ``abi`` crate.

    Returns ``(SERVICE_ABI_MAJOR, ABI_VERSION)``. Reading the crate source
    keeps the manifest ABI aligned with the table the module actually links.
    """
    path = pathlib.Path(sdk_root) / "abi" / "src" / "lib.rs"
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ManifestError(
            f"cannot read the SDK module ABI at {path}: {error}"
        ) from error
    match = ABI_VERSION_RE.search(text)
    if match is None:
        raise ManifestError(f"cannot find ABI_VERSION in {path}")
    minor = int(match.group(1))
    _u16(minor, "ABI_VERSION")
    return (SERVICE_ABI_MAJOR, minor)
