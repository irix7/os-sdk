// SPDX-License-Identifier: GPL-3.0-only
//! Process entry and exit.
//!
//! `_start` is deliberately tiny.  It captures the loader's stack pointer,
//! calls the image's `__irix7_user_main` (defined by [`crate::entry!`]) and
//! turns the return value into the exit syscall.  It does not set `$gp`: the
//! n32 target is compiled with `+noabicalls` and the linker script places the
//! image with absolute addressing.

use user_abi::SYS_EXIT;

/// Terminate the image and return `status` to the console.
///
/// The kernel removes the image, stack and heap mappings before printing the
/// exit code.  This function does not return.
pub fn exit(status: i32) -> ! {
    // SAFETY: exit is always a valid syscall for a user image.
    unsafe {
        crate::arch::syscall1(SYS_EXIT, status as usize);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Abort the image with status 134.
pub fn abort() -> ! {
    exit(134)
}

#[cfg(target_arch = "mips64")]
core::arch::global_asm!(
    r#"
    .set noreorder
    .set mips3

    .section .text._start, "ax", @progbits
    .align 4
    .globl _start
    .type _start, @function
_start:
    move    $a0, $sp
    jal     __irix7_user_main
    nop
    move    $a0, $v0
    li      $v0, 2
    syscall
1:  b       1b
    nop
    .size _start, .-_start
"#
);
