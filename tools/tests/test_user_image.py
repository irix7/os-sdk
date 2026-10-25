#!/usr/bin/env python3
"""Host-side validation of a built IRIX 7 static user image.

The test reads the ELF and raw target-2 image paths from the environment:

    IRIX7_USER_ELF=/tmp/user-hello.elf \
    IRIX7_USER_IMAGE=/tmp/user-hello.raw \
        python3 tools/tests/test_user_image.py

CI builds `examples/user-hello` and `examples/user-probe` with
`tools/irix7-user.py` and runs this test for each pair.  The test is deliberately
host-only: it validates the file the loader will read, not a running Indy.
"""

import importlib.util
import os
import pathlib
import struct
import sys
import unittest

TOOL = pathlib.Path(__file__).resolve().parents[1] / "irix7-user.py"
SPEC = importlib.util.spec_from_file_location("irix7_user", TOOL)
assert SPEC and SPEC.loader
irix7_user = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(irix7_user)

ELF_PATH = os.environ.get("IRIX7_USER_ELF")
IMAGE_PATH = os.environ.get("IRIX7_USER_IMAGE")
IMAGE_SIZE = irix7_user.IMAGE_SIZE


def load_elf():
    if not ELF_PATH:
        raise unittest.SkipTest("IRIX7_USER_ELF is not set")
    return pathlib.Path(ELF_PATH).read_bytes()


def load_image():
    if not IMAGE_PATH:
        raise unittest.SkipTest("IRIX7_USER_IMAGE is not set")
    return pathlib.Path(IMAGE_PATH).read_bytes()


def first_defined_symbol_offset(data):
    """Return the file offset of the first non-null defined symbol, or None."""
    parsed = irix7_user.parse_elf(data)
    symtab = next(
        (section for section in parsed["sections"] if section["type"] == 2), None
    )
    if symtab is None:
        return None
    entry_size = symtab["entsize"] or 16
    count = symtab["size"] // entry_size
    for index in range(1, count):
        base = symtab["offset"] + index * entry_size
        shndx = struct.unpack_from(">H", data, base + 14)[0]
        if shndx != 0:
            return base
    return None


class UserImageTest(unittest.TestCase):
    def setUp(self):
        self.elf = load_elf()
        self.summary = irix7_user.validate_user_elf(self.elf)

    def test_elf_is_big_endian_n32_mips3_exec(self):
        parsed = irix7_user.parse_elf(self.elf)
        header = parsed["header"]
        self.assertEqual(self.elf[4], 1, "ELFCLASS32")
        self.assertEqual(self.elf[5], 2, "ELFDATA2MSB")
        self.assertEqual(header["type"], 2, "ET_EXEC")
        self.assertEqual(header["machine"], 8, "EM_MIPS")
        flags = header["flags"]
        self.assertEqual(flags & 0x0000F000, 0, "n32 leaves the o32 ABI field zero")
        self.assertTrue(flags & 0x00000020, "EF_MIPS_ABI2 (n32)")
        self.assertEqual(flags & 0xF0000000, 0x20000000, "MIPS III")
        self.assertFalse(flags & 0x00000002, "static, not PIC")

    def test_entry_is_in_text(self):
        self.assertEqual(self.summary["entry"], 0x10000000)
        text = next(
            section for section in self.summary["sections"] if section["name"] == ".text"
        )
        self.assertEqual(text["addr"], self.summary["entry"])
        self.assertTrue(text["flags"] & 0x4, ".text is executable")

    def test_expected_sections_are_loadable(self):
        names = {section["name"] for section in self.summary["sections"]}
        self.assertIn(".text", names)
        self.assertIn(".rodata", names)
        self.assertIn(".data", names)
        for section in self.summary["sections"]:
            if section["size"] == 0:
                continue
            covered = any(
                segment["vaddr"] <= section["addr"]
                and section["addr"] + section["size"]
                <= segment["vaddr"] + segment["memsz"]
                for segment in self.summary["segments"]
            )
            self.assertTrue(covered, f"{section['name']} is not inside a PT_LOAD")

    def test_no_undefined_symbols(self):
        # validate_user_elf already rejects them; corrupt a defined symbol to
        # prove the check is live.
        parsed = irix7_user.parse_elf(self.elf)
        symtab = next(
            (section for section in parsed["sections"] if section["type"] == 2), None
        )
        self.assertIsNotNone(symtab, "the image must keep .symtab for the check")
        offset = first_defined_symbol_offset(self.elf)
        self.assertIsNotNone(offset)
        corrupted = bytearray(self.elf)
        struct.pack_into(">H", corrupted, offset + 14, 0)
        with self.assertRaises(irix7_user.ValidationError):
            irix7_user.validate_user_elf(bytes(corrupted))

    def test_elf_fits_loader_window(self):
        self.assertLessEqual(len(self.elf), IMAGE_SIZE)
        self.assertGreater(len(self.elf), 0)

    def test_image_is_padded_to_target_2_size(self):
        image = load_image()
        self.assertEqual(len(image), IMAGE_SIZE)
        self.assertEqual(image[: len(self.elf)], self.elf)
        self.assertEqual(image[len(self.elf) :], b"\0" * (IMAGE_SIZE - len(self.elf)))


class NegativeTest(unittest.TestCase):
    def setUp(self):
        self.elf = load_elf()

    def assert_rejected(self, data):
        with self.assertRaises(irix7_user.ValidationError):
            irix7_user.validate_user_elf(data)

    def test_rejects_little_endian(self):
        data = bytearray(self.elf)
        data[5] = 1
        self.assert_rejected(bytes(data))

    def test_rejects_o32_abi_flags(self):
        data = bytearray(self.elf)
        flags = struct.unpack_from(">I", data, 36)[0]
        struct.pack_into(">I", data, 36, flags & ~0x20)
        self.assert_rejected(bytes(data))

    def test_rejects_wrong_entry(self):
        data = bytearray(self.elf)
        struct.pack_into(">I", data, 24, 0)
        self.assert_rejected(bytes(data))

    def test_rejects_non_executable_type(self):
        data = bytearray(self.elf)
        struct.pack_into(">H", data, 16, 1)
        self.assert_rejected(bytes(data))

    def test_rejects_oversized_elf(self):
        self.assert_rejected(self.elf + b"\0" * (IMAGE_SIZE + 1 - len(self.elf)))

    def test_build_image_rejects_oversized_input(self):
        with self.assertRaises(irix7_user.ValidationError):
            irix7_user.build_image(b"\0" * (IMAGE_SIZE + 1))


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
