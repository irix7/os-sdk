// SPDX-License-Identifier: GPL-3.0-only
//! Raw syscall entry for the provisional n32 ABI.
//!
//! The number goes in `$v0`, arguments in `$a0`-`$a3`, the result comes back in
//! `$v0`.  The assembly marks every argument register as clobbered so the
//! compiler's register allocator cannot keep a value there across the trap.
//!
//! On a host build the entry points return `usize::MAX`; the host build exists
//! so `cargo clippy` and `cargo test` can inspect the non-architecture parts of
//! the crate.  A host binary is never a user image.

#[cfg(target_arch = "mips64")]
mod imp {
    /// Enter the kernel with no arguments.
    ///
    /// # Safety
    /// The caller must use a syscall number the kernel dispatches.
    #[inline(always)]
    pub unsafe fn syscall0(number: usize) -> usize {
        let result: usize;
        unsafe {
            core::arch::asm!(
                "syscall",
                inout("$2") number => result,
                out("$4") _,
                out("$5") _,
                out("$6") _,
                out("$7") _,
                options(nostack),
            );
        }
        result
    }

    /// Enter the kernel with one argument.
    ///
    /// # Safety
    /// The caller must use a syscall number the kernel dispatches and pass a
    /// valid argument for it.
    #[inline(always)]
    pub unsafe fn syscall1(number: usize, a0: usize) -> usize {
        let result: usize;
        unsafe {
            core::arch::asm!(
                "syscall",
                inout("$2") number => result,
                inout("$4") a0 => _,
                out("$5") _,
                out("$6") _,
                out("$7") _,
                options(nostack),
            );
        }
        result
    }

    /// Enter the kernel with two arguments.
    ///
    /// # Safety
    /// The caller must use a syscall number the kernel dispatches and pass
    /// valid arguments for it.
    #[inline(always)]
    pub unsafe fn syscall2(number: usize, a0: usize, a1: usize) -> usize {
        let result: usize;
        unsafe {
            core::arch::asm!(
                "syscall",
                inout("$2") number => result,
                inout("$4") a0 => _,
                inout("$5") a1 => _,
                out("$6") _,
                out("$7") _,
                options(nostack),
            );
        }
        result
    }

    /// Enter the kernel with three arguments.
    ///
    /// # Safety
    /// The caller must use a syscall number the kernel dispatches and pass
    /// valid arguments for it.
    #[inline(always)]
    pub unsafe fn syscall3(number: usize, a0: usize, a1: usize, a2: usize) -> usize {
        let result: usize;
        unsafe {
            core::arch::asm!(
                "syscall",
                inout("$2") number => result,
                inout("$4") a0 => _,
                inout("$5") a1 => _,
                inout("$6") a2 => _,
                out("$7") _,
                options(nostack),
            );
        }
        result
    }

    /// Enter the kernel with four arguments.
    ///
    /// # Safety
    /// The caller must use a syscall number the kernel dispatches and pass
    /// valid arguments for it.
    #[inline(always)]
    pub unsafe fn syscall4(number: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> usize {
        let result: usize;
        unsafe {
            core::arch::asm!(
                "syscall",
                inout("$2") number => result,
                inout("$4") a0 => _,
                inout("$5") a1 => _,
                inout("$6") a2 => _,
                inout("$7") a3 => _,
                options(nostack),
            );
        }
        result
    }
}

#[cfg(not(target_arch = "mips64"))]
mod imp {
    /// Host stub; see the module documentation.
    ///
    /// # Safety
    /// Always safe, but meaningless: there is no IRIX 7 kernel on the host.
    #[inline(always)]
    pub unsafe fn syscall0(_number: usize) -> usize {
        usize::MAX
    }

    /// Host stub; see the module documentation.
    ///
    /// # Safety
    /// Always safe, but meaningless: there is no IRIX 7 kernel on the host.
    #[inline(always)]
    pub unsafe fn syscall1(_number: usize, _a0: usize) -> usize {
        usize::MAX
    }

    /// Host stub; see the module documentation.
    ///
    /// # Safety
    /// Always safe, but meaningless: there is no IRIX 7 kernel on the host.
    #[inline(always)]
    pub unsafe fn syscall2(_number: usize, _a0: usize, _a1: usize) -> usize {
        usize::MAX
    }

    /// Host stub; see the module documentation.
    ///
    /// # Safety
    /// Always safe, but meaningless: there is no IRIX 7 kernel on the host.
    #[inline(always)]
    pub unsafe fn syscall3(_number: usize, _a0: usize, _a1: usize, _a2: usize) -> usize {
        usize::MAX
    }

    /// Host stub; see the module documentation.
    ///
    /// # Safety
    /// Always safe, but meaningless: there is no IRIX 7 kernel on the host.
    #[inline(always)]
    pub unsafe fn syscall4(
        _number: usize,
        _a0: usize,
        _a1: usize,
        _a2: usize,
        _a3: usize,
    ) -> usize {
        usize::MAX
    }
}

pub use imp::{syscall0, syscall1, syscall2, syscall3, syscall4};
