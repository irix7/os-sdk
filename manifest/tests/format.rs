// SPDX-License-Identifier: GPL-3.0-only
//! Codec, footer and ABI tests for the SDK manifest model.

use module_manifest::codec::{
    DecodeError, EncodeError, FORMAT_VERSION, FORMAT_VERSION_V1, MAGIC, MAX_RECORD,
};
use module_manifest::footer::{self, FooterError, ParseError, FOOTER_LEN};
use module_manifest::{
    Abi, Dependency, Export, Manifest, FOOTER_MAGIC, FOOTER_VERSION, SERVICE_ABI,
    SERVICE_ABI_MAJOR, SERVICE_ABI_MINOR,
};

const ABI: Abi = Abi::new(1, 0);

fn manifest() -> Manifest {
    Manifest::new("mesh.transport", "1.2.3", ABI)
        .with_entry_offset(0x40)
        .with_dependency(Dependency::new("mesh.codec", "1.0.0", [0x11; 32]))
        .with_dependency(Dependency::new("mcp", "0.1.3", [0x22; 32]))
        .with_export(Export::at("irix7.mesh.transport", 1, 0x80))
}

fn legacy_v1() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION_V1.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&12u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(b"m");
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(b"1.0");
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(b"i");
    out.extend_from_slice(&7u16.to_le_bytes());
    out
}

#[test]
fn round_trip_preserves_every_field() {
    let original = manifest();
    let encoded = original.encode().expect("encode");
    assert_eq!(Manifest::decode(&encoded).expect("decode"), original);
}

#[test]
fn format_is_little_endian_and_deterministic() {
    let encoded = manifest().encode().expect("encode");
    assert_eq!(&encoded[..4], b"I7MM");
    assert_eq!(&encoded[4..6], &FORMAT_VERSION.to_le_bytes());
    assert_eq!(&encoded[6..8], &1u16.to_le_bytes());
    assert_eq!(&encoded[8..10], &0u16.to_le_bytes());
    assert_eq!(&encoded[10..14], &0x40u32.to_le_bytes());
    assert_eq!(encoded, manifest().encode().expect("encode"));
}

#[test]
fn legacy_v1_record_decodes_with_zero_offsets() {
    let decoded = Manifest::decode(&legacy_v1()).expect("decode v1");
    assert_eq!(decoded.id.name, "m");
    assert_eq!(decoded.id.version, "1.0");
    assert_eq!(decoded.abi, Abi::new(1, 12));
    assert_eq!(decoded.entry_offset, 0);
    assert_eq!(decoded.exports, vec![Export::new("i", 7)]);
    assert_eq!(decoded.exports[0].offset, 0);

    let encoded = decoded.encode().expect("encode v2");
    assert_eq!(&encoded[4..6], &FORMAT_VERSION.to_le_bytes());
    assert_eq!(Manifest::decode(&encoded).expect("decode v2"), decoded);
}

#[test]
fn bad_magic_is_rejected() {
    let mut encoded = manifest().encode().expect("encode");
    encoded[0] = b'X';
    assert_eq!(Manifest::decode(&encoded), Err(DecodeError::BadMagic));
}

#[test]
fn unsupported_version_is_rejected() {
    let mut encoded = manifest().encode().expect("encode");
    encoded[4..6].copy_from_slice(&3u16.to_le_bytes());
    assert_eq!(
        Manifest::decode(&encoded),
        Err(DecodeError::UnsupportedVersion(3))
    );
}

#[test]
fn truncated_record_is_rejected() {
    let encoded = manifest().encode().expect("encode");
    for cut in 0..encoded.len() {
        assert!(
            Manifest::decode(&encoded[..cut]).is_err(),
            "record accepted at {cut} of {} bytes",
            encoded.len()
        );
    }

    let legacy = legacy_v1();
    for cut in 0..legacy.len() {
        assert!(
            Manifest::decode(&legacy[..cut]).is_err(),
            "v1 record accepted at {cut} of {} bytes",
            legacy.len()
        );
    }
}

#[test]
fn trailing_bytes_are_rejected() {
    let mut encoded = manifest().encode().expect("encode");
    encoded.push(0);
    assert_eq!(Manifest::decode(&encoded), Err(DecodeError::TrailingBytes));

    let mut legacy = legacy_v1();
    legacy.push(0);
    assert_eq!(Manifest::decode(&legacy), Err(DecodeError::TrailingBytes));
}

#[test]
fn invalid_utf8_is_rejected() {
    let mut encoded = manifest().encode().expect("encode");
    // Name starts after magic, format, ABI, entry offset and its u16 length:
    // 4 + 2 + 2 + 2 + 4 + 2 = 16.
    encoded[16] = 0xff;
    assert_eq!(Manifest::decode(&encoded), Err(DecodeError::InvalidUtf8));

    let mut legacy = legacy_v1();
    // Magic, format, ABI and the name length: 4 + 2 + 2 + 2 + 2 = 12.
    legacy[12] = 0xff;
    assert_eq!(Manifest::decode(&legacy), Err(DecodeError::InvalidUtf8));
}

#[test]
fn oversized_record_is_rejected_without_allocating() {
    let encoded = vec![0u8; MAX_RECORD + 1];
    assert_eq!(Manifest::decode(&encoded), Err(DecodeError::TooLarge));
}

#[test]
fn oversized_dependency_count_is_truncated_not_allocated() {
    let mut encoded = manifest().encode().expect("encode");
    // Overwrite the dependency count with u16::MAX; the reader must reject it
    // as truncated before allocating a matching vector.
    let header = 4 + 2 + 2 + 2 + 4;
    let name_len = u16::from_le_bytes([encoded[header], encoded[header + 1]]) as usize;
    let version_offset = header + 2 + name_len;
    let version_len =
        u16::from_le_bytes([encoded[version_offset], encoded[version_offset + 1]]) as usize;
    let count_offset = version_offset + 2 + version_len;
    encoded[count_offset..count_offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(Manifest::decode(&encoded).is_err());
}

#[test]
fn oversized_export_count_is_truncated_not_allocated() {
    let mut encoded = manifest().encode().expect("encode");
    // Locate the export count: header, name, version, deps.
    let mut pos = 4 + 2 + 2 + 2 + 4;
    let name_len = u16::from_le_bytes([encoded[pos], encoded[pos + 1]]) as usize;
    pos += 2 + name_len;
    let version_len = u16::from_le_bytes([encoded[pos], encoded[pos + 1]]) as usize;
    pos += 2 + version_len;
    let dep_count = u16::from_le_bytes([encoded[pos], encoded[pos + 1]]) as usize;
    pos += 2;
    for _ in 0..dep_count {
        let dep_name_len = u16::from_le_bytes([encoded[pos], encoded[pos + 1]]) as usize;
        pos += 2 + dep_name_len;
        let dep_version_len = u16::from_le_bytes([encoded[pos], encoded[pos + 1]]) as usize;
        pos += 2 + dep_version_len + 32;
    }
    encoded[pos..pos + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(Manifest::decode(&encoded).is_err());
}

#[test]
fn oversized_record_is_rejected_on_encode() {
    let mut manifest = Manifest::new("big", "1.0.0", ABI);
    for _ in 0..64 {
        manifest = manifest.with_dependency(Dependency::new("d".repeat(1000), "1.0.0", [0; 32]));
    }
    assert_eq!(manifest.encode(), Err(EncodeError::TooLarge));
}

#[test]
fn oversized_name_is_rejected_on_encode() {
    let manifest = Manifest::new("n".repeat(70_000), "1.0.0", ABI);
    assert_eq!(manifest.encode(), Err(EncodeError::FieldTooLong));
}

#[test]
fn errors_are_displayable() {
    for error in [EncodeError::TooLarge, EncodeError::FieldTooLong] {
        assert!(!format!("{error}").is_empty());
    }
    for error in [
        FooterError::TooLarge,
        FooterError::Encode(EncodeError::TooLarge),
    ] {
        assert!(!format!("{error}").is_empty());
    }
    for error in [
        ParseError::Reserved,
        ParseError::OutOfRange,
        ParseError::ManifestTooLarge,
        ParseError::UnsupportedVersion(2),
        ParseError::Manifest(DecodeError::BadMagic),
    ] {
        assert!(!format!("{error}").is_empty());
    }
}

#[test]
fn footer_round_trip_preserves_image_and_manifest() {
    let image = b"flat module bytes";
    let record_len = manifest().encode().expect("encode").len();
    let wrapped = footer::wrap(image, &manifest()).expect("wrap");
    assert_eq!(wrapped.len(), image.len() + record_len + FOOTER_LEN);
    assert_eq!(
        &wrapped[wrapped.len() - FOOTER_LEN..wrapped.len() - 12],
        &FOOTER_MAGIC
    );
    let (decoded, payload) = footer::parse(&wrapped).expect("parse").expect("footer");
    assert_eq!(decoded, manifest());
    assert_eq!(payload, image);
}

#[test]
fn footer_parses_a_legacy_v1_record() {
    let image = b"legacy image";
    let record = legacy_v1();
    let mut wrapped = image.to_vec();
    wrapped.extend_from_slice(&record);
    let offset = image.len() as u32;
    let length = record.len() as u32;
    wrapped.extend_from_slice(&FOOTER_MAGIC);
    wrapped.extend_from_slice(&FOOTER_VERSION.to_le_bytes());
    wrapped.extend_from_slice(&0u16.to_le_bytes());
    wrapped.extend_from_slice(&offset.to_le_bytes());
    wrapped.extend_from_slice(&length.to_le_bytes());

    let (decoded, payload) = footer::parse(&wrapped).expect("parse").expect("footer");
    assert_eq!(decoded.id.name, "m");
    assert_eq!(decoded.entry_offset, 0);
    assert_eq!(decoded.exports[0].offset, 0);
    assert_eq!(payload, image);
}

#[test]
fn standalone_image_has_no_footer() {
    assert_eq!(footer::parse(b"no footer here").expect("parse"), None);
    assert_eq!(footer::parse(b"").expect("parse"), None);
    assert_eq!(footer::parse(b"I7MF").expect("parse"), None);
}

#[test]
fn bad_footer_version_fails_closed() {
    let mut wrapped = footer::wrap(b"image", &manifest()).expect("wrap");
    let start = wrapped.len() - FOOTER_LEN;
    wrapped[start + 4..start + 6].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        footer::parse(&wrapped),
        Err(ParseError::UnsupportedVersion(2))
    );
}

#[test]
fn reserved_field_fails_closed() {
    let mut wrapped = footer::wrap(b"image", &manifest()).expect("wrap");
    let start = wrapped.len() - FOOTER_LEN;
    wrapped[start + 6] = 1;
    assert_eq!(footer::parse(&wrapped), Err(ParseError::Reserved));
}

#[test]
fn range_must_end_at_footer() {
    let mut wrapped = footer::wrap(b"image", &manifest()).expect("wrap");
    let start = wrapped.len() - FOOTER_LEN;
    wrapped[start + 12..start + 16].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(footer::parse(&wrapped), Err(ParseError::OutOfRange));

    let mut wrapped = footer::wrap(b"image", &manifest()).expect("wrap");
    let start = wrapped.len() - FOOTER_LEN;
    wrapped[start + 8..start + 12].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(footer::parse(&wrapped), Err(ParseError::OutOfRange));
}

#[test]
fn corrupt_manifest_is_rejected() {
    let mut wrapped = footer::wrap(b"image", &manifest()).expect("wrap");
    let start = wrapped.len() - FOOTER_LEN;
    let offset = u32::from_le_bytes([
        wrapped[start + 8],
        wrapped[start + 9],
        wrapped[start + 10],
        wrapped[start + 11],
    ]) as usize;
    wrapped[offset] = b'X';
    assert!(matches!(
        footer::parse(&wrapped),
        Err(ParseError::Manifest(_))
    ));
}

#[test]
fn oversized_manifest_is_rejected() {
    let mut wrapped = footer::wrap(b"image", &manifest()).expect("wrap");
    let start = wrapped.len() - FOOTER_LEN;
    wrapped[start + 12..start + 16].copy_from_slice(&((MAX_RECORD + 1) as u32).to_le_bytes());
    assert_eq!(footer::parse(&wrapped), Err(ParseError::ManifestTooLarge));
}

#[test]
fn footer_version_constant_matches_the_wire_format() {
    assert_eq!(FOOTER_VERSION, 1);
    assert_eq!(&FOOTER_MAGIC, b"I7MF");
}

#[test]
fn sdk_abi_is_generation_one_and_the_crate_revision() {
    assert_eq!(SERVICE_ABI_MAJOR, 1);
    assert_eq!(SERVICE_ABI_MINOR, module_abi::ABI_VERSION as u16);
    assert_eq!(SERVICE_ABI, Abi::new(1, module_abi::ABI_VERSION as u16));
    assert_eq!(Manifest::sdk("m", "1.0.0").abi, SERVICE_ABI);
}

#[test]
fn abi_compatibility_follows_the_os_rule() {
    assert!(Abi::new(1, 10).is_compatible_with(Abi::new(1, 11)));
    assert!(Abi::new(1, 11).is_compatible_with(Abi::new(1, 11)));
    assert!(!Abi::new(1, 12).is_compatible_with(Abi::new(1, 11)));
    assert!(!Abi::new(2, 0).is_compatible_with(Abi::new(1, 11)));
}
