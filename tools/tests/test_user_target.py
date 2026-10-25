#!/usr/bin/env python3
"""Host-side checks for the SDK's user target and linker contract.

These tests do not need a Rust toolchain; they guard the JSON and linker text
that the build tool consumes.
"""

import json
import pathlib
import re
import unittest

SDK = pathlib.Path(__file__).resolve().parents[2]
USER_TARGET = SDK / "targets" / "mips3-sgi-irix7-native-n32.json"
KERNEL_TARGET = SDK / "targets" / "mips3-sgi-irix7-n32.json"
USER_LINKER = SDK / "linker" / "user.ld"
USER_TOOL = SDK / "tools" / "irix7-user.py"


class UserTargetTest(unittest.TestCase):
    def setUp(self):
        self.target = json.loads(USER_TARGET.read_text())

    def test_target_is_native_n32(self):
        target = self.target
        self.assertEqual(target["arch"], "mips64")
        self.assertEqual(target["abi"], "n32")
        self.assertEqual(target["cpu"], "mips3")
        self.assertEqual(target["os"], "irix7")
        self.assertEqual(target["env"], "")
        self.assertEqual(target["target-family"], ["unix"])
        self.assertEqual(target["target-endian"], "big")
        self.assertEqual(target["target-pointer-width"], 32)
        self.assertEqual(target["llvm-abiname"], "n32")
        self.assertEqual(target["relocation-model"], "static")
        self.assertFalse(target["dynamic-linking"])
        self.assertFalse(target["position-independent-executables"])
        self.assertFalse(target["has-thread-local"])
        self.assertTrue(target["crt-objects-fallback"] == "false")
        self.assertEqual(target["panic-strategy"], "abort")

    def test_target_shares_the_kernel_data_layout(self):
        kernel = json.loads(KERNEL_TARGET.read_text())
        for field in (
            "data-layout",
            "features",
            "linker",
            "linker-flavor",
            "llvm-target",
            "llvm-args",
            "max-atomic-width",
            "target-pointer-width",
            "target-endian",
        ):
            self.assertEqual(self.target[field], kernel[field], field)
        self.assertNotIn("os", kernel, "the kernel target stays freestanding")

    def test_linker_is_static_and_starts_at_user_base(self):
        text = USER_LINKER.read_text()
        self.assertIn('OUTPUT_FORMAT("elf32-ntradbigmips")', text)
        self.assertIn("ENTRY(_start)", text)
        self.assertIn("KEEP(*(.text._start))", text)
        base = re.search(r"\.\s*=\s*0x([0-9A-Fa-f]+);", text)
        self.assertIsNotNone(base)
        self.assertEqual(int(base.group(1), 16), 0x10000000)
        self.assertIn("*(.bss)", text)

    def test_tool_packs_the_loader_window(self):
        text = USER_TOOL.read_text()
        self.assertIn("IMAGE_SIZE = 256 * 1024", text)
        self.assertIn("--undefined=_start", text)


if __name__ == "__main__":
    unittest.main()
