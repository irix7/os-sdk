// SPDX-License-Identifier: GPL-3.0-only
//! A small file and directory API over the bring-up descriptors.
//!
//! This is a convenience layer, not a POSIX file API.  The namespace is fixed:
//! `/`, `/etc`, `/dev`, `/dev/null`, `/dev/tty`, `/hello.txt` and `/etc/motd`.
//! There is no `openat`, no `O_CREAT`, no `lseek`, no permission model and no
//! general filesystem.  Descriptors are process-global because the bring-up
//! kernel has one process.

use crate::sys;
use user_abi::{Stat, O_RDONLY};

/// An open descriptor.
pub struct File {
    fd: i32,
}

impl File {
    /// Open a path.
    pub fn open(path: &[u8], flags: u32) -> Option<Self> {
        let fd = sys::open(path, flags);
        if fd < 0 {
            None
        } else {
            Some(Self { fd })
        }
    }

    /// Wrap an existing descriptor.
    pub fn from_raw(fd: i32) -> Option<Self> {
        if fd < 0 {
            None
        } else {
            Some(Self { fd })
        }
    }

    /// The raw descriptor number.
    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// Read into `buffer`.
    pub fn read(&mut self, buffer: &mut [u8]) -> isize {
        sys::read(self.fd, buffer)
    }

    /// Write from `buffer`.
    pub fn write(&mut self, buffer: &[u8]) -> isize {
        sys::write(self.fd, buffer)
    }

    /// Stat this descriptor.
    pub fn stat(&self) -> Option<Stat> {
        let mut stat = Stat::default();
        if sys::fstat(self.fd, &mut stat) {
            Some(stat)
        } else {
            None
        }
    }

    /// Close the descriptor.  Dropping the `File` does the same thing.
    pub fn close(mut self) {
        sys::close(self.fd);
        self.fd = -1;
    }

    /// Release ownership without closing the descriptor.
    pub fn into_raw(self) -> i32 {
        let fd = self.fd;
        core::mem::forget(self);
        fd
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if self.fd >= 0 {
            sys::close(self.fd);
            self.fd = -1;
        }
    }
}

/// Stat a path.
pub fn stat(path: &[u8]) -> Option<Stat> {
    let mut stat = Stat::default();
    if sys::stat(path, &mut stat) {
        Some(stat)
    } else {
        None
    }
}

/// Read the current working directory into `buffer`.
pub fn getcwd(buffer: &mut [u8]) -> Option<&[u8]> {
    sys::getcwd(buffer)
}

/// Change the current working directory.
pub fn chdir(path: &[u8]) -> bool {
    sys::chdir(path)
}

/// An open directory iterator.
pub struct Dir {
    fd: i32,
    index: usize,
}

impl Dir {
    /// Open a directory.  The bring-up namespace has `/`, `/etc` and `/dev`.
    pub fn open(path: &[u8]) -> Option<Self> {
        let file = File::open(path, O_RDONLY)?;
        Some(Self {
            fd: file.into_raw(),
            index: 0,
        })
    }

    /// Wrap an existing directory descriptor.
    pub fn from_raw(fd: i32) -> Option<Self> {
        if fd < 0 {
            None
        } else {
            Some(Self { fd, index: 0 })
        }
    }

    /// Read the next entry into `name` and return its type and name.
    pub fn next<'a>(&mut self, name: &'a mut [u8]) -> Option<(u8, &'a [u8])> {
        match sys::read_dir(self.fd, self.index, name) {
            sys::ReadDir::Entry { kind, length } => {
                self.index += 1;
                Some((kind, &name[..length]))
            }
            sys::ReadDir::End | sys::ReadDir::Error => None,
        }
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        if self.fd >= 0 {
            sys::close(self.fd);
            self.fd = -1;
        }
    }
}
