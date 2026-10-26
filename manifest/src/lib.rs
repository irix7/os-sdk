// SPDX-License-Identifier: GPL-3.0-only
//! Build-time manifest records and image footers for IRIX 7 modules.
//!
//! This crate is the SDK-side model for the wire format defined by
//! `crates/module-resolver` in [irix7/os](https://github.com/irix7/os)
//! (`docs/spec/modules.md` §8.1 and §8.2) for version 1. Version 2 is the
//! SDK loader-binding extension. It does not load or execute a module: the OS
//! resolver remains the host-side dependency resolver.
//!
//! A module image may append a version 2 manifest record and a [`footer`]:
//!
//! ```text
//! image bytes | I7MM record | I7MF footer
//! ```
//!
//! Version 2 adds the manager entry offset and an image-relative offset for
//! each exported function table, so a manager can enter a dependency and bind
//! an export without symbol relocation. The encoder emits version 2; the
//! decoder also accepts legacy version 1 records (entry and export offsets
//! default to zero).
//!
//! The record is little-endian and limited to [`MAX_RECORD`] (64 KiB). There
//! is no CRC: the SHA-256 of the complete image, including the record and the
//! footer, is the integrity boundary. A standalone image without the footer
//! remains valid, so an older module has no manifest and is treated as a
//! single self-contained module.
//!
//! A manifest is trailing data that the current kernel does not read, and the
//! kernel does not parse it. The ABI 12 loader passes entry and export offsets
//! explicitly, so resolution remains in the manager. Nothing in this crate
//! changes the entry at image offset 0.
//!
//! # ABI convention
//!
//! The record carries a `u16` ABI major and a `u16` ABI minor. These are the
//! service table's breaking generation and its additive revision:
//!
//! - [`SERVICE_ABI_MAJOR`] is `1`: the append-only table generation;
//! - [`SERVICE_ABI_MINOR`] is [`module_abi::ABI_VERSION`] (currently 12).
//!
//! A host accepts a module when the major is equal and the host minor is at
//! least the module minor, matching `module_resolver::Abi::is_compatible_with`.
//! Expressing the single table version as a minor is required by that rule: a
//! module built for table 10 must run on a host at table 12, which a
//! `major = 10, minor = 0` encoding would wrongly reject.
//!
//! # Example
//!
//! ```rust
//! use module_manifest::{Abi, Dependency, Export, Manifest, footer};
//!
//! let manifest = Manifest::new("mesh.transport", "1.2.3", Abi::new(1, 0))
//!     .with_dependency(Dependency::new("mesh.codec", "1.0.0", [0x11; 32]))
//!     .with_export(Export::at("irix7.mesh.transport", 1, 0x80));
//! let image = footer::wrap(b"flat module bytes", &manifest).expect("wrap");
//! assert_eq!(footer::parse(&image).expect("parse").expect("footer").1, b"flat module bytes");
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

pub mod codec;
pub mod footer;

use alloc::string::String;
use alloc::vec::Vec;

pub use codec::{DecodeError, EncodeError, FORMAT_VERSION, FORMAT_VERSION_V1, MAX_RECORD};
pub use footer::FOOTER_LEN;
pub use footer::{FORMAT_VERSION as FOOTER_VERSION, MAGIC as FOOTER_MAGIC};

/// A SHA-256 digest.
pub type Digest = [u8; 32];

/// Module service table major generation.
///
/// The table is append-only, so the generation has never changed: additive
/// services bump [`SERVICE_ABI_MINOR`].
pub const SERVICE_ABI_MAJOR: u16 = 1;

const _: () = assert!(module_abi::ABI_VERSION <= u16::MAX as u32);

/// Module service table additive revision understood by this SDK.
pub const SERVICE_ABI_MINOR: u16 = module_abi::ABI_VERSION as u16;

/// The module service table ABI this SDK builds against.
pub const SERVICE_ABI: Abi = Abi::new(SERVICE_ABI_MAJOR, SERVICE_ABI_MINOR);

/// Module ABI version.
///
/// This is the manifest's `abi` field, not the flat `module-abi`
/// [`module_abi::ABI_VERSION`] counter; see the crate documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Abi {
    /// Breaking ABI generation.
    pub major: u16,
    /// Additive ABI revision.
    pub minor: u16,
}

impl Abi {
    /// Construct an ABI version.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Whether a module built for `self` can run on `host`.
    ///
    /// The rule matches the OS resolver: equal major and a host minor at
    /// least as new as the module minor.
    pub const fn is_compatible_with(self, host: Abi) -> bool {
        self.major == host.major && self.minor <= host.minor
    }
}

/// Stable module name and exact version.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleId {
    /// Stable ASCII package name.
    pub name: String,
    /// Exact version selected from the package index.
    pub version: String,
}

impl ModuleId {
    /// Construct a module identity.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// One exact-version dependency and its expected image digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    /// Dependency identity.
    pub module: ModuleId,
    /// Required SHA-256 of the dependency image.
    pub sha256: Digest,
}

impl Dependency {
    /// Construct an exact dependency.
    pub fn new(name: impl Into<String>, version: impl Into<String>, sha256: Digest) -> Self {
        Self {
            module: ModuleId::new(name, version),
            sha256,
        }
    }
}

/// A versioned interface exported by a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    /// Interface name.
    pub interface: String,
    /// Interface version.
    pub version: u16,
    /// Image-relative offset of the interface's C function table, or zero for
    /// an interface-only declaration.
    pub offset: u32,
}

impl Export {
    /// Construct an export declaration without a table offset.
    pub fn new(interface: impl Into<String>, version: u16) -> Self {
        Self::at(interface, version, 0)
    }

    /// Construct an export declaration at an image-relative table offset.
    pub fn at(interface: impl Into<String>, version: u16, offset: u32) -> Self {
        Self {
            interface: interface.into(),
            version,
            offset,
        }
    }
}

/// Module manifest, excluding the image bytes themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Module identity.
    pub id: ModuleId,
    /// ABI the image was built for.
    pub abi: Abi,
    /// Image-relative offset of the manager entry, or zero for a legacy
    /// `_start(const Services *)` entry.
    pub entry_offset: u32,
    /// Exact runtime dependencies.
    pub dependencies: Vec<Dependency>,
    /// Interfaces this module provides.
    pub exports: Vec<Export>,
}

impl Manifest {
    /// Construct a manifest with no dependencies or exports.
    pub fn new(name: impl Into<String>, version: impl Into<String>, abi: Abi) -> Self {
        Self {
            id: ModuleId::new(name, version),
            abi,
            entry_offset: 0,
            dependencies: Vec::new(),
            exports: Vec::new(),
        }
    }

    /// Construct a manifest for the SDK's current service table.
    pub fn sdk(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self::new(name, version, SERVICE_ABI)
    }

    /// Set the image-relative manager entry offset.
    pub fn with_entry_offset(mut self, entry_offset: u32) -> Self {
        self.entry_offset = entry_offset;
        self
    }

    /// Add an exact dependency declaration.
    pub fn with_dependency(mut self, dependency: Dependency) -> Self {
        self.dependencies.push(dependency);
        self
    }

    /// Add an export declaration.
    pub fn with_export(mut self, export: Export) -> Self {
        self.exports.push(export);
        self
    }
}

/// The expected digest field for a dependency.
pub const DEPENDENCY_DIGEST_LEN: usize = codec::DEPENDENCY_DIGEST_LEN;
