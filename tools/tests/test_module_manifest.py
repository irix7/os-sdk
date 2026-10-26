#!/usr/bin/env python3
"""Host-side checks for the module manifest codec and the build tool.

The v1 vectors in ``testdata/`` were produced by the OS encoder in
``crates/module-resolver`` (irix7/os at 5a7696e) and remain decode-only
compatibility vectors. The v2 vectors are the SDK's canonical bytes. These
tests fail if the Python and Rust implementations drift; ``manifest/tests``
checks the Rust model against the same files.
"""

import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

SDK = pathlib.Path(__file__).resolve().parents[2]
TOOLS = SDK / "tools"
TESTDATA = SDK / "testdata"

sys.path.insert(0, str(TOOLS))

import irix7_manifest as codec  # noqa: E402

SDK_ABI = codec.sdk_module_abi(SDK)


def load_tool():
    spec = importlib.util.spec_from_file_location(
        "irix7_module", TOOLS / "irix7-module.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MODULE_TOOL = load_tool()


def load_vector(name):
    fields = {}
    path = TESTDATA / name
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, _, value = line.partition("=")
        if key in fields:
            raise AssertionError(f"duplicate {key} in {path}")
        fields[key] = value
    return fields


def manifest_from_vector(fields):
    dependencies = tuple(
        codec.Dependency(
            fields[f"dep{index}_name"],
            fields[f"dep{index}_version"],
            bytes.fromhex(fields[f"dep{index}_sha256"]),
        )
        for index in range(int(fields["dep_count"]))
    )
    exports = tuple(
        codec.Export(
            fields[f"export{index}_interface"],
            int(fields[f"export{index}_version"]),
            int(fields.get(f"export{index}_offset", 0)),
        )
        for index in range(int(fields["export_count"]))
    )
    return codec.Manifest(
        name=fields["name"],
        version=fields["version"],
        abi_major=int(fields["abi_major"]),
        abi_minor=int(fields["abi_minor"]),
        dependencies=dependencies,
        exports=exports,
        entry_offset=int(fields.get("entry_offset", 0)),
    )


def sample_manifest(**overrides):
    values = {
        "name": "mesh.transport",
        "version": "1.2.3",
        "abi_major": SDK_ABI[0],
        "abi_minor": SDK_ABI[1],
        "dependencies": (
            codec.Dependency("mesh.codec", "1.0.0", b"\x11" * 32),
        ),
        "exports": (codec.Export("irix7.mesh.transport", 1),),
        "entry_offset": 0,
    }
    values.update(overrides)
    return codec.Manifest(**values)


def offset_manifest(**overrides):
    values = {
        "name": "mesh.transport",
        "version": "1.2.3",
        "abi_major": SDK_ABI[0],
        "abi_minor": SDK_ABI[1],
        "dependencies": (
            codec.Dependency("mesh.codec", "1.0.0", b"\x11" * 32),
        ),
        "exports": (codec.Export("irix7.mesh.transport", 1, 0x80),),
        "entry_offset": 0x40,
    }
    values.update(overrides)
    return codec.Manifest(**values)


def cli_args(**overrides):
    values = {
        "manifest_json": None,
        "name": None,
        "version": None,
        "abi_major": None,
        "abi_minor": None,
        "entry_offset": None,
        "depends": None,
        "export": None,
    }
    values.update(overrides)
    return argparse.Namespace(**values)


class VectorTest(unittest.TestCase):
    def check(self, name):
        fields = load_vector(name)
        manifest = manifest_from_vector(fields)
        format_version = int(fields["format"])

        record = bytes.fromhex(fields["record_hex"])
        self.assertEqual(len(record), int(fields["record_len"]))
        self.assertEqual(codec.decode_manifest(record), manifest)
        if format_version == codec.LEGACY_FORMAT_VERSION:
            self.assertEqual(manifest.entry_offset, 0)
            self.assertTrue(all(export.offset == 0 for export in manifest.exports))
        else:
            self.assertEqual(format_version, codec.FORMAT_VERSION)
            self.assertEqual(codec.encode_manifest(manifest), record)

        image = bytes.fromhex(fields["image_hex"])
        codec.validate_image_offsets(manifest, len(image))
        wrapped = bytes.fromhex(fields["wrapped_hex"])
        self.assertEqual(len(wrapped), int(fields["wrapped_len"]))
        if format_version == codec.FORMAT_VERSION:
            self.assertEqual(codec.wrap_image(image, manifest), wrapped)
        self.assertEqual(
            hashlib.sha256(wrapped).hexdigest(), fields["wrapped_sha256"]
        )

        footer = wrapped[-codec.FOOTER_LEN :]
        self.assertEqual(footer[:4], codec.FOOTER_MAGIC)
        self.assertEqual(int.from_bytes(footer[4:6], "little"), codec.FOOTER_VERSION)
        self.assertEqual(footer[6:8], b"\x00\x00")
        self.assertEqual(int.from_bytes(footer[8:12], "little"), len(image))
        self.assertEqual(int.from_bytes(footer[12:16], "little"), len(record))

        self.assertEqual(codec.parse_wrapped(wrapped), (manifest, image))
        self.assertIsNone(codec.parse_wrapped(image))

    def test_full_v1_vector_decodes(self):
        self.check("manifest-v1.txt")

    def test_minimal_v1_vector_decodes(self):
        self.check("manifest-minimal.txt")

    def test_full_v2_vector_matches_both_implementations(self):
        self.check("manifest-v2.txt")

    def test_minimal_v2_vector_matches_both_implementations(self):
        self.check("manifest-v2-minimal.txt")

    def test_vector_manifest_passes_sdk_validation(self):
        self.check("manifest-v2.txt")
        codec.validate_manifest(
            manifest_from_vector(load_vector("manifest-v2.txt")), SDK_ABI
        )


class CodecTest(unittest.TestCase):
    def test_little_endian_header(self):
        record = codec.encode_manifest(offset_manifest())
        self.assertEqual(record[:4], b"I7MM")
        self.assertEqual(int.from_bytes(record[4:6], "little"), 2)
        self.assertEqual(int.from_bytes(record[6:8], "little"), SDK_ABI[0])
        self.assertEqual(int.from_bytes(record[8:10], "little"), SDK_ABI[1])
        self.assertEqual(int.from_bytes(record[10:14], "little"), 0x40)

    def test_round_trip_preserves_every_field(self):
        manifest = offset_manifest()
        self.assertEqual(codec.decode_manifest(codec.encode_manifest(manifest)), manifest)

    def test_version_1_record_decodes_with_zero_offsets(self):
        record = (
            b"I7MM"
            + (1).to_bytes(2, "little")
            + (1).to_bytes(2, "little")
            + (12).to_bytes(2, "little")
            + (1).to_bytes(2, "little")
            + b"m"
            + (3).to_bytes(2, "little")
            + b"1.0"
            + (0).to_bytes(2, "little")
            + (1).to_bytes(2, "little")
            + (1).to_bytes(2, "little")
            + b"i"
            + (7).to_bytes(2, "little")
        )
        manifest = codec.decode_manifest(record)
        self.assertEqual(manifest.name, "m")
        self.assertEqual(manifest.version, "1.0")
        self.assertEqual(manifest.entry_offset, 0)
        self.assertEqual(manifest.exports, (codec.Export("i", 7),))
        self.assertEqual(manifest.exports[0].offset, 0)
        self.assertEqual(codec.decode_manifest(codec.encode_manifest(manifest)), manifest)

    def test_standalone_image_has_no_footer(self):
        for image in (b"", b"no footer here", b"I7MF", b"\x00" * 15):
            self.assertIsNone(codec.parse_wrapped(image))

    def test_bad_magic_is_rejected(self):
        record = bytearray(codec.encode_manifest(sample_manifest()))
        record[0] = ord("X")
        with self.assertRaisesRegex(codec.ManifestError, "bad magic"):
            codec.decode_manifest(bytes(record))

    def test_unsupported_record_version_is_rejected(self):
        record = bytearray(codec.encode_manifest(sample_manifest()))
        record[4:6] = (3).to_bytes(2, "little")
        with self.assertRaisesRegex(codec.ManifestError, "version 3"):
            codec.decode_manifest(bytes(record))

    def test_truncated_record_is_rejected(self):
        record = codec.encode_manifest(sample_manifest())
        for cut in range(len(record)):
            with self.assertRaises(codec.ManifestError):
                codec.decode_manifest(record[:cut])

    def test_trailing_bytes_are_rejected(self):
        record = codec.encode_manifest(sample_manifest()) + b"\x00"
        with self.assertRaisesRegex(codec.ManifestError, "trailing"):
            codec.decode_manifest(record)

    def test_invalid_utf8_is_rejected(self):
        record = bytearray(codec.encode_manifest(sample_manifest()))
        # Magic, format, ABI, entry offset and the name length: 4 + 2 + 2 + 2 + 4 + 2.
        record[16] = 0xFF
        with self.assertRaisesRegex(codec.ManifestError, "UTF-8"):
            codec.decode_manifest(bytes(record))

    def test_oversized_record_is_rejected(self):
        with self.assertRaisesRegex(codec.ManifestError, "limit is 65536"):
            codec.decode_manifest(b"\x00" * (codec.MAX_RECORD + 1))

    def test_impossible_dependency_count_is_rejected(self):
        record = bytearray(codec.encode_manifest(sample_manifest()))
        header = 4 + 2 + 2 + 2 + 4
        name_len = int.from_bytes(record[header : header + 2], "little")
        version_offset = header + 2 + name_len
        version_len = int.from_bytes(record[version_offset : version_offset + 2], "little")
        count_offset = version_offset + 2 + version_len
        record[count_offset : count_offset + 2] = (0xFFFF).to_bytes(2, "little")
        with self.assertRaisesRegex(codec.ManifestError, "truncated"):
            codec.decode_manifest(bytes(record))

    def test_oversized_record_is_rejected_on_encode(self):
        manifest = sample_manifest(
            dependencies=tuple(
                codec.Dependency("d" * 1000, "1.0.0", b"\x00" * 32)
                for _ in range(64)
            )
        )
        with self.assertRaisesRegex(codec.ManifestError, "limit is 65536"):
            codec.encode_manifest(manifest)

    def test_oversized_string_is_rejected_on_encode(self):
        manifest = sample_manifest(name="n" * 70_000)
        with self.assertRaisesRegex(codec.ManifestError, "65535"):
            codec.encode_manifest(manifest)

    def test_digest_length_is_rejected_on_encode(self):
        manifest = sample_manifest(
            dependencies=(codec.Dependency("a", "1.0.0", b"\x00" * 31),)
        )
        with self.assertRaisesRegex(codec.ManifestError, "32 bytes"):
            codec.encode_manifest(manifest)


class FooterTest(unittest.TestCase):
    def wrapped(self):
        return bytearray(codec.wrap_image(b"flat module bytes", sample_manifest()))

    def test_bad_footer_version_fails_closed(self):
        image = self.wrapped()
        start = len(image) - codec.FOOTER_LEN
        image[start + 4 : start + 6] = (2).to_bytes(2, "little")
        with self.assertRaisesRegex(codec.ManifestError, "footer version 2"):
            codec.parse_wrapped(bytes(image))

    def test_reserved_field_fails_closed(self):
        image = self.wrapped()
        image[-codec.FOOTER_LEN + 6] = 1
        with self.assertRaisesRegex(codec.ManifestError, "reserved"):
            codec.parse_wrapped(bytes(image))

    def test_range_must_end_at_footer(self):
        for offset, length in ((0, None), (None, 0), (0, 0)):
            image = self.wrapped()
            start = len(image) - codec.FOOTER_LEN
            if offset is not None:
                image[start + 8 : start + 12] = offset.to_bytes(4, "little")
            if length is not None:
                image[start + 12 : start + 16] = length.to_bytes(4, "little")
            with self.assertRaisesRegex(codec.ManifestError, "out of bounds"):
                codec.parse_wrapped(bytes(image))

    def test_corrupt_manifest_is_rejected(self):
        image = self.wrapped()
        start = len(image) - codec.FOOTER_LEN
        offset = int.from_bytes(image[start + 8 : start + 12], "little")
        image[offset] = ord("X")
        with self.assertRaisesRegex(codec.ManifestError, "bad magic"):
            codec.parse_wrapped(bytes(image))

    def test_oversized_footer_length_is_rejected(self):
        image = self.wrapped()
        start = len(image) - codec.FOOTER_LEN
        image[start + 12 : start + 16] = (codec.MAX_RECORD + 1).to_bytes(4, "little")
        with self.assertRaisesRegex(codec.ManifestError, "limit is 65536"):
            codec.parse_wrapped(bytes(image))


class ValidationTest(unittest.TestCase):
    def validate(self, manifest, sdk_abi=(1, 12)):
        codec.validate_manifest(manifest, sdk_abi)

    def test_accepts_a_valid_manifest(self):
        self.validate(sample_manifest())
        self.validate(offset_manifest())

    def test_accepts_an_older_minor(self):
        self.validate(sample_manifest(abi_minor=1), sdk_abi=(1, 11))

    def test_rejects_a_newer_minor(self):
        with self.assertRaisesRegex(codec.ManifestError, "newer than"):
            self.validate(sample_manifest(abi_minor=12), sdk_abi=(1, 11))

    def test_rejects_an_unknown_major(self):
        with self.assertRaisesRegex(codec.ManifestError, "unsupported ABI major 2"):
            self.validate(sample_manifest(abi_major=2), sdk_abi=(1, 11))

    def test_rejects_abi_out_of_u16(self):
        with self.assertRaisesRegex(codec.ManifestError, "outside 0..65535"):
            self.validate(sample_manifest(abi_minor=0x10000), sdk_abi=(1, 0xFFFF))

    def test_rejects_a_duplicate_dependency(self):
        dependency = codec.Dependency("a", "1.0.0", b"\x00" * 32)
        with self.assertRaisesRegex(codec.ManifestError, "duplicate dependency"):
            self.validate(sample_manifest(dependencies=(dependency, dependency)))

    def test_rejects_conflicting_dependency_versions(self):
        dependencies = (
            codec.Dependency("a", "1.0.0", b"\x00" * 32),
            codec.Dependency("a", "1.0.1", b"\x01" * 32),
        )
        with self.assertRaisesRegex(codec.ManifestError, "conflicting dependency versions"):
            self.validate(sample_manifest(dependencies=dependencies))

    def test_rejects_a_duplicate_export(self):
        export = codec.Export("irix7.mesh.transport", 1)
        with self.assertRaisesRegex(codec.ManifestError, "duplicate export"):
            self.validate(sample_manifest(exports=(export, export)))

    def test_allows_two_versions_of_one_interface(self):
        self.validate(
            sample_manifest(
                exports=(
                    codec.Export("irix7.mesh.transport", 1),
                    codec.Export("irix7.mesh.transport", 2),
                )
            )
        )

    def test_rejects_unsupported_name_characters(self):
        for name in ("has space", "name@version", "name:digest", "name/slash", ""):
            with self.assertRaises(codec.ManifestError):
                self.validate(sample_manifest(name=name))

    def test_rejects_unsupported_version_characters(self):
        with self.assertRaisesRegex(codec.ManifestError, "unsupported characters"):
            self.validate(sample_manifest(version="1.0.0 space"))

    def test_rejects_a_misaligned_entry_offset(self):
        with self.assertRaisesRegex(codec.ManifestError, "aligned to 4 bytes"):
            self.validate(sample_manifest(entry_offset=2))

    def test_rejects_a_misaligned_export_offset(self):
        with self.assertRaisesRegex(codec.ManifestError, "aligned to 4 bytes"):
            self.validate(
                sample_manifest(exports=(codec.Export("i", 1, 2),))
            )

    def test_image_offsets_must_fit_the_flattened_image(self):
        manifest = offset_manifest()
        codec.validate_image_offsets(manifest, 0x100)
        with self.assertRaisesRegex(codec.ManifestError, "entry_offset 64"):
            codec.validate_image_offsets(manifest, 64)
        with self.assertRaisesRegex(codec.ManifestError, "offset 128"):
            codec.validate_image_offsets(manifest, 128)

    def test_zero_offsets_are_not_image_checked(self):
        codec.validate_image_offsets(sample_manifest(), 0)


class SpecTest(unittest.TestCase):
    def test_dependency_spec(self):
        dependency = codec.parse_dependency_spec("mesh.codec@1.0.0:" + "AB" * 32)
        self.assertEqual(dependency.name, "mesh.codec")
        self.assertEqual(dependency.version, "1.0.0")
        self.assertEqual(dependency.sha256, bytes.fromhex("ab" * 32))

    def test_dependency_spec_rejects_missing_separators(self):
        for spec in ("mesh.codec", "mesh.codec@1.0.0", "@1.0.0:" + "11" * 32, "mesh.codec@:"):
            with self.assertRaises(codec.ManifestError):
                codec.parse_dependency_spec(spec)

    def test_dependency_spec_rejects_bad_digests(self):
        for digest in ("11" * 31, "11" * 33, "gg" * 32):
            with self.assertRaisesRegex(codec.ManifestError, "64 hexadecimal"):
                codec.parse_dependency_spec(f"mesh.codec@1.0.0:{digest}")
        with self.assertRaisesRegex(codec.ManifestError, "name@version:sha256"):
            codec.parse_dependency_spec("mesh.codec@1.0.0:")

    def test_export_spec(self):
        export = codec.parse_export_spec("irix7.mesh.transport@1")
        self.assertEqual(export, codec.Export("irix7.mesh.transport", 1))
        export = codec.parse_export_spec("irix7.mesh.transport@1:128")
        self.assertEqual(export, codec.Export("irix7.mesh.transport", 1, 128))
        with self.assertRaisesRegex(codec.ManifestError, "aligned to 4 bytes"):
            codec.parse_export_spec("irix7.mesh.transport@1:2")
        with self.assertRaisesRegex(codec.ManifestError, "non-numeric offset"):
            codec.parse_export_spec("irix7.mesh.transport@1:abc")

    def test_export_spec_rejects_bad_versions(self):
        specs = (
            "irix7.mesh.transport",
            "irix7.mesh.transport@",
            "@1",
            "x@-1",
            "x@65536",
            "x@one",
        )
        for spec in specs:
            with self.assertRaises(codec.ManifestError):
                codec.parse_export_spec(spec)


class JsonTest(unittest.TestCase):
    def test_round_trip(self):
        manifest = offset_manifest()
        self.assertEqual(codec.manifest_from_json(codec.manifest_to_json(manifest)), manifest)

    def test_rejects_unknown_keys(self):
        document = codec.manifest_to_json(sample_manifest())
        document["dependencies"] = []
        with self.assertRaisesRegex(codec.ManifestError, "unknown key"):
            codec.manifest_from_json(document)

    def test_rejects_unknown_dependency_keys(self):
        document = codec.manifest_to_json(sample_manifest())
        document["depends"][0]["url"] = "https://example.invalid"
        with self.assertRaisesRegex(codec.ManifestError, "depends\\[0\\].*unknown key"):
            codec.manifest_from_json(document)

    def test_rejects_missing_fields(self):
        for field in ("name", "version", "abi_major", "abi_minor"):
            document = codec.manifest_to_json(sample_manifest())
            del document[field]
            with self.assertRaisesRegex(codec.ManifestError, f"missing '{field}'"):
                codec.manifest_from_json(document)

    def test_rejects_legacy_format_documents(self):
        document = codec.manifest_to_json(sample_manifest())
        document["format"] = 1
        with self.assertRaisesRegex(codec.ManifestError, "legacy and decode-only"):
            codec.manifest_from_json(document)

    def test_rejects_an_unsupported_format(self):
        document = codec.manifest_to_json(sample_manifest())
        document["format"] = 3
        with self.assertRaisesRegex(codec.ManifestError, "unsupported manifest format 3"):
            codec.manifest_from_json(document)

        document = codec.manifest_to_json(sample_manifest())
        document["format"] = 2.0
        with self.assertRaisesRegex(codec.ManifestError, "must be an integer"):
            codec.manifest_from_json(document)

    def test_rejects_misaligned_offsets(self):
        document = codec.manifest_to_json(sample_manifest())
        document["entry_offset"] = 2
        with self.assertRaisesRegex(codec.ManifestError, "aligned to 4 bytes"):
            codec.manifest_from_json(document)

        document = codec.manifest_to_json(offset_manifest())
        document["exports"][0]["offset"] = 130
        with self.assertRaisesRegex(codec.ManifestError, "aligned to 4 bytes"):
            codec.manifest_from_json(document)

    def test_rejects_wrong_types(self):
        cases = (
            ("abi_major", "1", "abi_major must be an integer"),
            ("abi_minor", -1, "outside 0..65535"),
            ("entry_offset", "64", "entry_offset must be an integer"),
            ("version", 3, "manifest version must be a string"),
            ("depends", {}, "depends must be a list"),
        )
        for field, value, message in cases:
            document = codec.manifest_to_json(sample_manifest())
            document[field] = value
            with self.assertRaisesRegex(codec.ManifestError, message):
                codec.manifest_from_json(document)

    def test_rejects_missing_dependency_fields(self):
        document = codec.manifest_to_json(sample_manifest())
        del document["depends"][0]["sha256"]
        with self.assertRaisesRegex(codec.ManifestError, "depends\\[0\\] is missing 'sha256'"):
            codec.manifest_from_json(document)

    def test_rejects_a_bad_digest(self):
        document = codec.manifest_to_json(sample_manifest())
        document["depends"][0]["sha256"] = "xyz"
        with self.assertRaisesRegex(codec.ManifestError, "64 hexadecimal"):
            codec.manifest_from_json(document)

    def test_rejects_non_object_documents(self):
        for document in (None, [], "manifest"):
            with self.assertRaisesRegex(codec.ManifestError, "must be an object"):
                codec.manifest_from_json(document)


class SdkAbiTest(unittest.TestCase):
    def test_matches_the_abi_crate(self):
        text = (SDK / "abi" / "src" / "lib.rs").read_text()
        match = re.search(r"pub const ABI_VERSION: u32 = (\d+);", text)
        self.assertIsNotNone(match)
        self.assertEqual(SDK_ABI, (1, int(match.group(1))))

    def test_rejects_a_missing_abi_crate(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(codec.ManifestError, "cannot read"):
                codec.sdk_module_abi(directory)


class BuildManifestTest(unittest.TestCase):
    def build(self, args):
        return MODULE_TOOL.build_manifest(args, SDK)

    def test_no_manifest_options_returns_none(self):
        self.assertIsNone(self.build(cli_args()))

    def test_inline_flags_build_the_manifest(self):
        manifest = self.build(
            cli_args(
                name="mesh.transport",
                version="1.2.3",
                abi_major=SDK_ABI[0],
                abi_minor=SDK_ABI[1],
                entry_offset=0x40,
                depends=["mesh.codec@1.0.0:" + "11" * 32],
                export=["irix7.mesh.transport@1:128"],
            )
        )
        self.assertEqual(manifest, offset_manifest())

    def test_json_and_flags_encode_identically(self):
        expected = self.build(
            cli_args(
                name="mesh.transport",
                version="1.2.3",
                abi_major=SDK_ABI[0],
                abi_minor=SDK_ABI[1],
                entry_offset=0x40,
                depends=["mesh.codec@1.0.0:" + "11" * 32],
                export=["irix7.mesh.transport@1:128"],
            )
        )
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "manifest.json"
            path.write_text(json.dumps(codec.manifest_to_json(offset_manifest())))
            manifest = self.build(cli_args(manifest_json=str(path)))
        self.assertEqual(
            codec.encode_manifest(manifest), codec.encode_manifest(expected)
        )

    def test_rejects_mixed_sources(self):
        with self.assertRaisesRegex(codec.ManifestError, "cannot be combined"):
            self.build(
                cli_args(manifest_json="manifest.json", name="mesh.transport")
            )

    def test_rejects_missing_fields(self):
        for missing in ("version", "abi_major", "abi_minor"):
            args = cli_args(
                name="mesh.transport",
                version="1.2.3",
                abi_major=SDK_ABI[0],
                abi_minor=SDK_ABI[1],
            )
            setattr(args, missing, None)
            with self.assertRaisesRegex(codec.ManifestError, "missing --"):
                self.build(args)

    def test_rejects_invalid_dependencies(self):
        with self.assertRaises(codec.ManifestError):
            self.build(
                cli_args(
                    name="mesh.transport",
                    version="1.2.3",
                    abi_major=SDK_ABI[0],
                    abi_minor=SDK_ABI[1],
                    depends=["mesh.codec@1.0.0"],
                )
            )


@unittest.skipUnless(os.name == "posix", "the fake toolchain is a POSIX script set")
class BuildToolTest(unittest.TestCase):
    """End-to-end checks of the tool with a fake cargo and objcopy."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.fixture = bytes(range(256))
        self.fixture_path = self.root / "fixture.bin"
        self.fixture_path.write_bytes(self.fixture)
        self.output = self.root / "hello-sdk.module"
        self.cargo_manifest = self.root / "Cargo.toml"
        self.cargo_manifest.write_text('[package]\nname = "hello-sdk"\n')
        target_dir = self.root / "target"
        elf = target_dir / "mips3-sgi-irix7-n32" / "release" / "hello-sdk"

        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        rustc = bin_dir / "rustc"
        rustc.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            "  --print) echo /nonexistent;;\n"
            "  -vV) echo 'host: x86_64-unknown-linux-gnu';;\n"
            "  *) exit 1;;\n"
            "esac\n"
        )
        cargo = bin_dir / "cargo"
        cargo.write_text(
            "#!/bin/sh\n"
            'if [ "$1" = "metadata" ]; then\n'
            "  cat <<'JSON'\n"
            '{"packages":[{"name":"hello-sdk","targets":[{"name":"hello-sdk","kind":["bin"]}]}],'
            f'"target_directory":"{target_dir}"}}\n'
            "JSON\n"
            "  exit 0\n"
            "fi\n"
            'if [ "$1" = "build" ]; then\n'
            f'  mkdir -p "{elf.parent}"\n'
            f'  : > "{elf}"\n'
            "  exit 0\n"
            "fi\n"
            "exit 1\n"
        )
        objcopy = bin_dir / "llvm-objcopy"
        objcopy.write_text('#!/bin/sh\ncp "$FAKE_IMAGE" "$4"\n')
        for script in (rustc, cargo, objcopy):
            script.chmod(0o755)

        self.env = os.environ.copy()
        self.env["PATH"] = str(bin_dir) + os.pathsep + self.env["PATH"]
        self.env["FAKE_IMAGE"] = str(self.fixture_path)

    def run_tool(self, *arguments):
        command = [
            sys.executable,
            str(TOOLS / "irix7-module.py"),
            "--manifest-path",
            str(self.cargo_manifest),
            "--package",
            "hello-sdk",
            "--output",
            str(self.output),
            "--json",
            *arguments,
        ]
        return subprocess.run(
            command,
            cwd=self.root,
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_no_manifest_keeps_the_old_image_and_output(self):
        result = self.run_tool()
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(set(data), {"module", "size", "sha256"})
        self.assertEqual(self.output.read_bytes(), self.fixture)
        self.assertEqual(data["size"], len(self.fixture))
        self.assertEqual(data["sha256"], hashlib.sha256(self.fixture).hexdigest())

    def test_manifest_json_appends_the_record_and_hashes_the_image(self):
        manifest = offset_manifest()
        path = self.root / "manifest.json"
        path.write_text(json.dumps(codec.manifest_to_json(manifest)))
        result = self.run_tool("--manifest-json", str(path))
        self.assertEqual(result.returncode, 0, result.stderr)

        expected = codec.wrap_image(self.fixture, manifest)
        data = json.loads(result.stdout)
        self.assertEqual(self.output.read_bytes(), expected)
        self.assertEqual(data["size"], len(expected))
        self.assertEqual(data["sha256"], hashlib.sha256(expected).hexdigest())
        self.assertEqual(data["image_size"], len(self.fixture))
        self.assertEqual(
            data["manifest_size"], len(expected) - len(self.fixture) - codec.FOOTER_LEN
        )
        self.assertEqual(data["footer_size"], codec.FOOTER_LEN)
        self.assertEqual(data["manifest"], codec.manifest_to_json(manifest))
        self.assertEqual(codec.parse_wrapped(self.output.read_bytes()), (manifest, self.fixture))

    def test_inline_flags_match_the_json_manifest(self):
        manifest = offset_manifest()
        result = self.run_tool(
            "--name",
            manifest.name,
            "--version",
            manifest.version,
            "--abi-major",
            str(manifest.abi_major),
            "--abi-minor",
            str(manifest.abi_minor),
            "--entry-offset",
            str(manifest.entry_offset),
            "--depends",
            "mesh.codec@1.0.0:" + "11" * 32,
            "--export",
            "irix7.mesh.transport@1:128",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.output.read_bytes(), codec.wrap_image(self.fixture, manifest)
        )

    def test_invalid_manifest_fails_before_writing(self):
        result = self.run_tool(
            "--name",
            "has space",
            "--version",
            "1.2.3",
            "--abi-major",
            str(SDK_ABI[0]),
            "--abi-minor",
            str(SDK_ABI[1]),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("manifest:", result.stderr)
        self.assertFalse(self.output.exists())

    def test_entry_offset_beyond_the_image_is_rejected(self):
        result = self.run_tool(
            "--name",
            "m",
            "--version",
            "1.0.0",
            "--abi-major",
            str(SDK_ABI[0]),
            "--abi-minor",
            str(SDK_ABI[1]),
            "--entry-offset",
            "1024",
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("does not fit", result.stderr)
        self.assertIsNone(codec.parse_wrapped(self.output.read_bytes()))


if __name__ == "__main__":
    unittest.main()
