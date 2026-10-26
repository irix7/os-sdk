// SPDX-License-Identifier: GPL-3.0-only
//! Cross-implementation and compatibility vectors.
//!
//! `testdata/manifest-v1.txt` and `testdata/manifest-minimal.txt` were
//! produced by `module_resolver::footer::wrap` in irix7/os at 5a7696e. They are
//! decode-only compatibility vectors: the SDK encoder emits version 2.
//!
//! `testdata/manifest-v2.txt` and `testdata/manifest-v2-minimal.txt` are the
//! SDK's version 2 vectors. The Rust and Python encoders must produce exactly
//! these bytes.

use module_manifest::codec::{FORMAT_VERSION, FORMAT_VERSION_V1};
use module_manifest::{footer, Abi, Dependency, Export, Manifest};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

struct Vector {
    fields: BTreeMap<String, String>,
}

impl Vector {
    fn load(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("testdata")
            .join(name);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let mut fields = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("bad vector line {line:?}"));
            assert!(
                fields.insert(key.to_string(), value.to_string()).is_none(),
                "duplicate vector key {key}"
            );
        }
        Self { fields }
    }

    fn get(&self, key: &str) -> &str {
        self.fields
            .get(key)
            .unwrap_or_else(|| panic!("vector is missing {key}"))
    }

    fn optional_u32(&self, key: &str) -> u32 {
        self.fields
            .get(key)
            .map(|value| value.parse().unwrap_or_else(|_| panic!("{key}")))
            .unwrap_or(0)
    }

    fn u16(&self, key: &str) -> u16 {
        self.get(key).parse().unwrap_or_else(|_| panic!("{key}"))
    }

    fn usize(&self, key: &str) -> usize {
        self.get(key).parse().unwrap_or_else(|_| panic!("{key}"))
    }

    fn bytes(&self, key: &str) -> Vec<u8> {
        let text = self.get(key);
        assert_eq!(text.len() % 2, 0, "{key} is not whole bytes");
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    fn digest(&self, key: &str) -> [u8; 32] {
        let bytes = self.bytes(key);
        assert_eq!(bytes.len(), 32, "{key}");
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&bytes);
        digest
    }

    fn manifest(&self) -> Manifest {
        let mut manifest = Manifest::new(
            self.get("name"),
            self.get("version"),
            Abi::new(self.u16("abi_major"), self.u16("abi_minor")),
        )
        .with_entry_offset(self.optional_u32("entry_offset"));
        for index in 0..self.usize("dep_count") {
            manifest = manifest.with_dependency(Dependency::new(
                self.get(&format!("dep{index}_name")),
                self.get(&format!("dep{index}_version")),
                self.digest(&format!("dep{index}_sha256")),
            ));
        }
        for index in 0..self.usize("export_count") {
            manifest = manifest.with_export(Export::at(
                self.get(&format!("export{index}_interface")),
                self.u16(&format!("export{index}_version")),
                self.optional_u32(&format!("export{index}_offset")),
            ));
        }
        manifest
    }

    fn check(&self) {
        let format = self.u16("format");
        let manifest = self.manifest();
        let record = self.bytes("record_hex");
        assert_eq!(record.len(), self.usize("record_len"));

        match format {
            FORMAT_VERSION_V1 => {
                assert_eq!(Manifest::decode(&record).expect("decode v1"), manifest);
                assert_eq!(manifest.entry_offset, 0);
                assert!(manifest.exports.iter().all(|export| export.offset == 0));
            }
            FORMAT_VERSION => {
                assert_eq!(manifest.encode().expect("encode"), record);
                assert_eq!(Manifest::decode(&record).expect("decode v2"), manifest);
            }
            other => panic!("unexpected vector format {other}"),
        }

        let image = self.bytes("image_hex");
        let wrapped = self.bytes("wrapped_hex");
        assert_eq!(wrapped.len(), self.usize("wrapped_len"));
        if format == FORMAT_VERSION {
            assert_eq!(footer::wrap(&image, &manifest).expect("wrap"), wrapped);
        }
        let footer_bytes = &wrapped[wrapped.len() - footer::FOOTER_LEN..];
        assert_eq!(&footer_bytes[..4], &footer::MAGIC);
        assert_eq!(
            u16::from_le_bytes([footer_bytes[4], footer_bytes[5]]),
            footer::FORMAT_VERSION
        );
        assert_eq!(&footer_bytes[6..8], &[0, 0]);

        let (decoded, payload) = footer::parse(&wrapped)
            .expect("parse")
            .expect("wrapped image has a footer");
        assert_eq!(decoded, manifest);
        assert_eq!(payload, image);
    }
}

#[test]
fn full_v1_vector_decodes() {
    Vector::load("manifest-v1.txt").check();
}

#[test]
fn minimal_v1_vector_decodes() {
    Vector::load("manifest-minimal.txt").check();
}

#[test]
fn full_v2_vector_matches_both_implementations() {
    Vector::load("manifest-v2.txt").check();
}

#[test]
fn minimal_v2_vector_matches_both_implementations() {
    Vector::load("manifest-v2-minimal.txt").check();
}
