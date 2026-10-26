#!/usr/bin/env python3
"""Host-side checks for the SDK Wasm guest example.

Build the guest first::

    cargo build --manifest-path examples/wasm-hello/Cargo.toml --release \
      --target wasm32-unknown-unknown -Z build-std=core

Set ``IRIX7_WASM_GUEST`` to the resulting file, or use the default path. These
checks parse the WebAssembly sections; they do not run the guest, because there
is no v1 `i7host` host yet.
"""

import os
import pathlib
import re
import unittest

SDK = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_GUEST = (
    SDK
    / "examples"
    / "wasm-hello"
    / "target"
    / "wasm32-unknown-unknown"
    / "release"
    / "wasm_hello_guest.wasm"
)
GUEST = pathlib.Path(os.environ.get("IRIX7_WASM_GUEST", DEFAULT_GUEST))

I32 = 0x7F
I64 = 0x7E


def uleb(data, pos):
    result = 0
    shift = 0
    while True:
        if pos >= len(data):
            raise ValueError("truncated LEB128")
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def read_name(data, pos):
    length, pos = uleb(data, pos)
    end = pos + length
    if end > len(data):
        raise ValueError("truncated name")
    return data[pos:end].decode("utf-8"), end


def parse_module(data):
    if data[:4] != b"\x00asm":
        raise ValueError("not a WebAssembly module")
    version = int.from_bytes(data[4:8], "little")
    types = []
    imports = []
    exports = []
    pos = 8
    while pos < len(data):
        section_id = data[pos]
        pos += 1
        size, pos = uleb(data, pos)
        payload = data[pos : pos + size]
        if len(payload) != size:
            raise ValueError("truncated section")
        pos += size
        if section_id == 1:
            count, cursor = uleb(payload, 0)
            for _ in range(count):
                if payload[cursor] != 0x60:
                    raise ValueError("unsupported type form")
                cursor += 1
                param_count, cursor = uleb(payload, cursor)
                params = list(payload[cursor : cursor + param_count])
                cursor += param_count
                result_count, cursor = uleb(payload, cursor)
                results = list(payload[cursor : cursor + result_count])
                cursor += result_count
                types.append((params, results))
        elif section_id == 2:
            count, cursor = uleb(payload, 0)
            for _ in range(count):
                module, cursor = read_name(payload, cursor)
                field, cursor = read_name(payload, cursor)
                kind = payload[cursor]
                cursor += 1
                type_index = None
                if kind == 0:
                    type_index, cursor = uleb(payload, cursor)
                elif kind == 1:
                    cursor += 1  # reftype
                    flags = payload[cursor]
                    cursor += 1
                    _, cursor = uleb(payload, cursor)
                    if flags & 1:
                        _, cursor = uleb(payload, cursor)
                elif kind == 2:
                    flags = payload[cursor]
                    cursor += 1
                    _, cursor = uleb(payload, cursor)
                    if flags & 1:
                        _, cursor = uleb(payload, cursor)
                elif kind == 3:
                    cursor += 2
                elif kind == 4:
                    cursor += 1
                    _, cursor = uleb(payload, cursor)
                else:
                    raise ValueError(f"unsupported import kind {kind}")
                imports.append(
                    {
                        "module": module,
                        "field": field,
                        "kind": kind,
                        "type_index": type_index,
                    }
                )
        elif section_id == 7:
            count, cursor = uleb(payload, 0)
            for _ in range(count):
                name, cursor = read_name(payload, cursor)
                kind = payload[cursor]
                cursor += 1
                index, cursor = uleb(payload, cursor)
                exports.append({"name": name, "kind": kind, "index": index})
    return {"version": version, "types": types, "imports": imports, "exports": exports}


class WasmGuestTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not GUEST.exists():
            raise unittest.SkipTest(f"{GUEST} is missing; build the guest first")
        cls.module = parse_module(GUEST.read_bytes())

    def functions(self):
        return {
            item["field"]: item
            for item in self.module["imports"]
            if item["kind"] == 0
        }

    def test_is_a_version_1_module(self):
        self.assertEqual(self.module["version"], 1)

    def test_imports_only_i7host(self):
        modules = {item["module"] for item in self.module["imports"]}
        self.assertEqual(modules, {"i7host"})

    def test_has_no_wasi_imports(self):
        for item in self.module["imports"]:
            self.assertNotIn("wasi", item["module"].lower())
            self.assertNotIn("wasi", item["field"].lower())

    def test_declares_the_core_host_calls(self):
        names = set(self.functions())
        self.assertLessEqual(
            {"host_version", "host_submit", "host_poll"}, names
        )

    def test_host_submit_passes_a_guest_slice_indirectly(self):
        function = self.functions()["host_submit"]
        params, results = self.module["types"][function["type_index"]]
        # op: u32, a0..a3: u64, GuestSlice: pointer to { ptr: u32, len: u32 }.
        self.assertEqual(params, [I32, I64, I64, I64, I64, I32])
        self.assertEqual(results, [I64])

    def test_host_calls_use_32_bit_offsets(self):
        function = self.functions()["host_poll"]
        params, results = self.module["types"][function["type_index"]]
        self.assertEqual(params, [I32, I32])
        self.assertEqual(results, [I32])

    def test_exports_run_and_memory(self):
        functions = [
            item for item in self.module["exports"] if item["kind"] == 0
        ]
        memories = [item for item in self.module["exports"] if item["kind"] == 2]
        self.assertTrue(
            any(item["name"] == "run" for item in functions),
            "the host calls the exported run function",
        )
        self.assertTrue(
            any(item["name"] == "memory" for item in memories),
            "the host reads and writes the exported memory",
        )

    def test_abi_version_is_one(self):
        text = (SDK / "wasm-abi" / "src" / "lib.rs").read_text()
        match = re.search(r"pub const HOST_ABI_VERSION: u32 = (\d+);", text)
        self.assertIsNotNone(match)
        self.assertEqual(int(match.group(1)), 1)


if __name__ == "__main__":
    unittest.main()
