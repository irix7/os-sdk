// SPDX-License-Identifier: GPL-3.0-only
//! Developer SDK for IRIX 7 loadable modules.
//!
//! A module is a flat image linked at `0x8840_0000`, fetched over HTTPS, checked against
//! a SHA-256 digest and called by the kernel.  This crate wraps the C service table with
//! a [`Context`], the [`entry!`] macro and a [`panic_handler!`] macro; the ABI itself comes
//! from [`module_abi`] in the IRIX 7 OS repository.
//!
//! The kernel and the module must agree on the ABI version.  [`Context::new`] rejects a
//! table older than [`ABI_VERSION`].

#![no_std]

pub use module_abi::{ModuleVtable, Services, ABI_VERSION, MODULE_BASE, MODULE_MAX};

/// A module's view of the kernel services.
#[derive(Clone, Copy)]
pub struct Context {
    services: &'static Services,
}

impl Context {
    /// Validate and store the table passed to `_start`.
    ///
    /// # Safety
    /// `services` must point to the kernel's service table for the lifetime of the
    /// module.  The kernel passes it to `_start` and never moves it.
    pub unsafe fn new(services: *const Services) -> Option<Self> {
        let services: &'static Services = unsafe { services.as_ref()? };
        if services.version < ABI_VERSION {
            return None;
        }
        Some(Self { services })
    }

    /// Write bytes to the serial and telnet consoles.
    pub fn write(&self, data: &[u8]) {
        (self.services.write)(data.as_ptr(), data.len());
    }

    /// Write a string.
    pub fn print(&self, text: &str) {
        self.write(text.as_bytes());
    }

    /// Write a string and CRLF.
    pub fn println(&self, text: &str) {
        self.print(text);
        self.write(b"\r\n");
    }

    /// Monotonic milliseconds since boot.
    pub fn clock_ms(&self) -> u64 {
        (self.services.clock_ms)()
    }

    /// The shared machine status block (IP, MAC, RAM, uptime, date).
    pub fn status<'a>(&self, out: &'a mut [u8]) -> &'a [u8] {
        let n = unsafe { (self.services.status)(out.as_mut_ptr(), out.len()) };
        if n > 0 {
            &out[..n as usize]
        } else {
            &out[..0]
        }
    }

    /// Format one of the shared information blocks: 0 status, 1 net, 2 mem, 3 time.
    pub fn info<'a>(&self, kind: u32, out: &'a mut [u8]) -> &'a [u8] {
        let n = unsafe { (self.services.info)(kind, out.as_mut_ptr(), out.len()) };
        if n > 0 {
            &out[..n as usize]
        } else {
            &out[..0]
        }
    }

    /// Copy BGR shadow pixels from `offset`.
    pub fn screen_read<'a>(&self, offset: u32, out: &'a mut [u8]) -> &'a [u8] {
        let n = unsafe { (self.services.screen_read)(offset, out.as_mut_ptr(), out.len()) };
        if n > 0 {
            &out[..n as usize]
        } else {
            &out[..0]
        }
    }

    /// Allocate from the kernel heap.  The heap never frees.
    ///
    /// # Safety
    /// The returned memory is uninitialised; the caller must treat it as such.
    pub unsafe fn alloc(&self, size: usize, align: usize) -> *mut u8 {
        unsafe { (self.services.alloc)(size, align) }
    }

    /// Listen on `port` in `slot` (0 or 1).
    pub fn tcp_listen(&self, slot: u32, port: u16) -> bool {
        unsafe { (self.services.tcp_listen)(slot, port) == 0 }
    }

    /// 0 closed, 1 listening, 2 established, 3 closing.
    pub fn tcp_state(&self, slot: u32) -> u32 {
        (self.services.tcp_state)(slot)
    }

    /// Receive into `out`; returns bytes, 0 for none, negative for no socket.
    pub fn tcp_recv(&self, slot: u32, out: &mut [u8]) -> isize {
        unsafe { (self.services.tcp_recv)(slot, out.as_mut_ptr(), out.len()) }
    }

    /// Send `data`; returns bytes accepted or a negative error.
    pub fn tcp_send(&self, slot: u32, data: &[u8]) -> isize {
        unsafe { (self.services.tcp_send)(slot, data.as_ptr(), data.len()) }
    }

    /// Send a FIN.
    pub fn tcp_close(&self, slot: u32) {
        (self.services.tcp_close)(slot);
    }

    /// Send a RST.
    pub fn tcp_abort(&self, slot: u32) {
        (self.services.tcp_abort)(slot);
    }

    /// Read a PROM environment value by name.
    pub fn config_get<'a>(&self, name: &str, out: &'a mut [u8]) -> Option<&'a [u8]> {
        let n = unsafe {
            (self.services.config_get)(name.as_ptr(), name.len(), out.as_mut_ptr(), out.len())
        };
        if n >= 0 {
            Some(&out[..n as usize])
        } else {
            None
        }
    }

    /// Restart the machine from the PROM.  Does not return.
    pub fn reset(&self) -> ! {
        (self.services.reset)();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Halt the machine.  Does not return.
    pub fn poweroff(&self) -> ! {
        (self.services.poweroff)();
        loop {
            core::hint::spin_loop();
        }
    }
}

/// Define the kernel entry point.
///
/// The supplied function must have the signature
/// `fn(&Context) -> *const ModuleVtable`.
#[macro_export]
macro_rules! entry {
    ($run:path) => {
        #[no_mangle]
        pub unsafe extern "C" fn _start(
            services: *const $crate::Services,
        ) -> *const $crate::ModuleVtable {
            match $crate::Context::new(services) {
                Some(context) => $run(&context),
                None => core::ptr::null(),
            }
        }
    };
}

/// Install a minimal halt-on-panic handler.
#[macro_export]
macro_rules! panic_handler {
    () => {
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            loop {
                core::hint::spin_loop();
            }
        }
    };
}

// The module links only core and compiler_builtins; provide the byte
// primitives the compiler may emit for array initialisation and copies.
//
/// # Safety
/// `dst` must be valid for writes of `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn memset(
    dst: *mut core::ffi::c_void,
    c: i32,
    n: usize,
) -> *mut core::ffi::c_void {
    let dst = dst as *mut u8;
    let mut i = 0;
    while i < n {
        unsafe {
            *dst.add(i) = c as u8;
        }
        i += 1;
    }
    dst as *mut core::ffi::c_void
}

/// # Safety
/// `dst` must be valid for writes of `n` bytes and `src` for reads of `n`
/// bytes; the regions must not overlap.
#[no_mangle]
pub unsafe extern "C" fn memcpy(
    dst: *mut core::ffi::c_void,
    src: *const core::ffi::c_void,
    n: usize,
) -> *mut core::ffi::c_void {
    let d = dst as *mut u8;
    let s = src as *const u8;
    let mut i = 0;
    while i < n {
        unsafe {
            *d.add(i) = *s.add(i);
        }
        i += 1;
    }
    dst
}

/// # Safety
/// `dst` must be valid for writes of `n` bytes and `src` for reads of `n`
/// bytes; the regions may overlap.
#[no_mangle]
pub unsafe extern "C" fn memmove(
    dst: *mut core::ffi::c_void,
    src: *const core::ffi::c_void,
    n: usize,
) -> *mut core::ffi::c_void {
    let d = dst as *mut u8;
    let s = src as *const u8;
    if (d as usize) < (s as usize) {
        return unsafe { memcpy(dst, src, n) };
    }
    let mut i = n;
    while i > 0 {
        i -= 1;
        unsafe {
            *d.add(i) = *s.add(i);
        }
    }
    dst
}

/// # Safety
/// `a` and `b` must be valid for reads of `n` bytes.
#[no_mangle]
pub unsafe extern "C" fn memcmp(
    a: *const core::ffi::c_void,
    b: *const core::ffi::c_void,
    n: usize,
) -> i32 {
    let a = a as *const u8;
    let b = b as *const u8;
    let mut i = 0;
    while i < n {
        let x = unsafe { *a.add(i) };
        let y = unsafe { *b.add(i) };
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}
