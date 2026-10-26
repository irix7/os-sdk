// SPDX-License-Identifier: GPL-3.0-only
//! Deterministic manifest record codec.
//!
//! The record is a sidecar payload, not the module image itself. The image
//! digest covers the bytes that contain it. Version 2 is little-endian:
//!
//! ```text
//! I7MM | u16 format=2 | u16 abi major | u16 abi minor
//!      | u32 entry_offset
//!      | u16 name_len | name | u16 version_len | version
//!      | u16 dep_count | (u16 name_len, name, u16 version_len, version, 32-byte digest)*
//!      | u16 export_count | (u16 interface_len, interface, u16 export_version, u32 table_offset)*
//! ```
//!
//! Version 1 is still decoded for legacy images:
//!
//! ```text
//! I7MM | u16 format=1 | u16 abi major | u16 abi minor
//!      | u16 name_len | name | u16 version_len | version
//!      | u16 dep_count | (u16 name_len, name, u16 version_len, version, 32-byte digest)*
//!      | u16 export_count | (u16 interface_len, interface, u16 export_version)*
//! ```
//!
//! A version 1 record has no manager entry and no export table offsets; both
//! decode as zero. The encoder only emits version 2.
//!
//! No CRC is included: the enclosing image digest is the integrity boundary.
//! A record is limited to 64 KiB so a corrupt dependency count cannot make a
//! reader allocate without a bound.

use alloc::string::{FromUtf8Error, String};
use alloc::vec::Vec;
use core::fmt;

use crate::{Abi, Dependency, Export, Manifest, ModuleId};

/// Manifest record magic.
pub const MAGIC: [u8; 4] = *b"I7MM";
/// Current manifest record format version.
pub const FORMAT_VERSION: u16 = 2;
/// Legacy version 1 record format, accepted for decoding.
pub const FORMAT_VERSION_V1: u16 = 1;
/// Maximum encoded manifest record length.
pub const MAX_RECORD: usize = 64 * 1024;

/// Manifest encoding failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// The encoded record would exceed [`MAX_RECORD`].
    TooLarge,
    /// A string or vector length does not fit the u16 field.
    FieldTooLong,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("manifest record exceeds 64 KiB"),
            Self::FieldTooLong => f.write_str("manifest field exceeds u16 length"),
        }
    }
}

/// Manifest decoding failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Input does not contain a complete header.
    TooShort,
    /// The magic is not `I7MM`.
    BadMagic,
    /// The format version is not supported.
    UnsupportedVersion(u16),
    /// A declared field runs past the end of the input.
    Truncated,
    /// The input is larger than [`MAX_RECORD`].
    TooLarge,
    /// The record has bytes after the declared exports.
    TrailingBytes,
    /// A string field is not valid UTF-8.
    InvalidUtf8,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => f.write_str("manifest record is too short"),
            Self::BadMagic => f.write_str("manifest record has bad magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "manifest record version {version} is unsupported")
            }
            Self::Truncated => f.write_str("manifest record is truncated"),
            Self::TooLarge => f.write_str("manifest record exceeds 64 KiB"),
            Self::TrailingBytes => f.write_str("manifest record has trailing bytes"),
            Self::InvalidUtf8 => f.write_str("manifest string is not UTF-8"),
        }
    }
}

impl From<FromUtf8Error> for DecodeError {
    fn from(_: FromUtf8Error) -> Self {
        Self::InvalidUtf8
    }
}

impl Manifest {
    /// Encode this manifest as a version 2 record.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        push_u16(&mut out, FORMAT_VERSION)?;
        push_u16(&mut out, self.abi.major)?;
        push_u16(&mut out, self.abi.minor)?;
        push_u32(&mut out, self.entry_offset);
        push_string(&mut out, &self.id.name)?;
        push_string(&mut out, &self.id.version)?;
        push_u16(&mut out, checked_len(self.dependencies.len())?)?;
        for dependency in &self.dependencies {
            push_string(&mut out, &dependency.module.name)?;
            push_string(&mut out, &dependency.module.version)?;
            out.extend_from_slice(&dependency.sha256);
        }
        push_u16(&mut out, checked_len(self.exports.len())?)?;
        for export in &self.exports {
            push_string(&mut out, &export.interface)?;
            push_u16(&mut out, export.version)?;
            push_u32(&mut out, export.offset);
        }
        if out.len() > MAX_RECORD {
            return Err(EncodeError::TooLarge);
        }
        Ok(out)
    }

    /// Decode one complete version 2 record or a legacy version 1 record.
    ///
    /// A version 1 record decodes with `entry_offset == 0` and every export
    /// offset equal to zero.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_RECORD {
            return Err(DecodeError::TooLarge);
        }
        let mut reader = Reader { bytes, pos: 0 };
        if reader.remaining() < 4 {
            return Err(DecodeError::TooShort);
        }
        if reader.take(4)? != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        match reader.u16()? {
            FORMAT_VERSION_V1 => Self::decode_v1(&mut reader),
            FORMAT_VERSION => Self::decode_v2(&mut reader),
            version => Err(DecodeError::UnsupportedVersion(version)),
        }
    }

    fn decode_v1(reader: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let abi = Abi::new(reader.u16()?, reader.u16()?);
        let name = reader.string()?;
        let module_version = reader.string()?;
        let dependencies = decode_dependencies(reader)?;
        let export_count = reader.u16()? as usize;
        // Every export is at least two u16 lengths.
        if export_count > reader.remaining() / 4 {
            return Err(DecodeError::Truncated);
        }
        let mut exports = Vec::with_capacity(export_count);
        for _ in 0..export_count {
            let interface = reader.string()?;
            exports.push(Export::new(interface, reader.u16()?));
        }
        if reader.remaining() != 0 {
            return Err(DecodeError::TrailingBytes);
        }
        Ok(Self {
            id: ModuleId::new(name, module_version),
            abi,
            entry_offset: 0,
            dependencies,
            exports,
        })
    }

    fn decode_v2(reader: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let abi = Abi::new(reader.u16()?, reader.u16()?);
        let entry_offset = reader.u32()?;
        let name = reader.string()?;
        let module_version = reader.string()?;
        let dependencies = decode_dependencies(reader)?;
        let export_count = reader.u16()? as usize;
        // Every export is at least two u16 lengths plus a u32 offset.
        if export_count > reader.remaining() / 8 {
            return Err(DecodeError::Truncated);
        }
        let mut exports = Vec::with_capacity(export_count);
        for _ in 0..export_count {
            let interface = reader.string()?;
            let version = reader.u16()?;
            let offset = reader.u32()?;
            exports.push(Export::at(interface, version, offset));
        }
        if reader.remaining() != 0 {
            return Err(DecodeError::TrailingBytes);
        }
        Ok(Self {
            id: ModuleId::new(name, module_version),
            abi,
            entry_offset,
            dependencies,
            exports,
        })
    }
}

fn decode_dependencies(reader: &mut Reader<'_>) -> Result<Vec<Dependency>, DecodeError> {
    let count = reader.u16()? as usize;
    // Every dependency is at least two u16 lengths plus a 32-byte digest.
    // Reject an impossible count before reserving a vector for it.
    if count > reader.remaining() / 36 {
        return Err(DecodeError::Truncated);
    }
    let mut dependencies = Vec::with_capacity(count);
    for _ in 0..count {
        let name = reader.string()?;
        let version = reader.string()?;
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(reader.take(32)?);
        dependencies.push(Dependency::new(name, version, sha256));
    }
    Ok(dependencies)
}

fn checked_len(len: usize) -> Result<u16, EncodeError> {
    u16::try_from(len).map_err(|_| EncodeError::FieldTooLong)
}

fn push_u16(out: &mut Vec<u8>, value: u16) -> Result<(), EncodeError> {
    out.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), EncodeError> {
    let len = u16::try_from(value.len()).map_err(|_| EncodeError::FieldTooLong)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(len).ok_or(DecodeError::Truncated)?;
        if end > self.bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn string(&mut self) -> Result<String, DecodeError> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }
}

/// The expected digest field for a dependency.
pub const DEPENDENCY_DIGEST_LEN: usize = 32;
