// SPDX-License-Identifier: GPL-3.0-only
//! Clock and sleep helpers over the provisional syscalls.
//!
//! `CLOCK_REALTIME` reads the RTC/NTP wall clock and returns zero if the kernel
//! has not learned the date yet.  `CLOCK_MONOTONIC` counts CP0 Count at the
//! Indy's 33 MHz rate.  `nanosleep` busy-waits in the kernel; there is no
//! scheduler to block on.

use crate::sys;
use user_abi::{Timespec, CLOCK_MONOTONIC, CLOCK_REALTIME};

/// Read a clock.
pub fn clock(clock: i32) -> Option<Timespec> {
    sys::clock_gettime(clock)
}

/// Read the monotonic clock.
pub fn monotonic() -> Option<Timespec> {
    clock(CLOCK_MONOTONIC)
}

/// Read the wall clock.
pub fn realtime() -> Option<Timespec> {
    clock(CLOCK_REALTIME)
}

/// Monotonic milliseconds since boot, saturating on overflow.
pub fn monotonic_millis() -> Option<i64> {
    monotonic().map(|value| value.as_millis())
}

/// Sleep for `seconds` and `nanoseconds` of monotonic time.
pub fn sleep(seconds: i64, nanoseconds: i32) -> bool {
    if !(0..1_000_000_000).contains(&nanoseconds) {
        return false;
    }
    sys::nanosleep(&Timespec::new(seconds.max(0), nanoseconds))
}
