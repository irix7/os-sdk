// SPDX-License-Identifier: GPL-3.0-only
//! Developer SDK for IRIX 7 loadable modules.
//!
//! A module is a flat image linked at `0x8840_0000`, fetched over HTTPS, checked against
//! a SHA-256 digest and called by the kernel.  This crate wraps the C service table with
//! a [`Context`], the [`entry!`]/[`entry_with_context!`] macros and a
//! [`panic_handler!`] macro; the ABI itself comes from [`module_abi`] in the IRIX 7 OS
//! repository.
//!
//! The kernel and the module must agree on the ABI version.  [`Context::new`] rejects a
//! table older than [`ABI_VERSION`].  ABI 12 appends the loader services; a downloadable
//! manager uses the [`Context::module_arena`], [`Context::module_load`],
//! [`Context::module_execute`], [`Context::module_find`], [`Context::module_find_export`],
//! [`Context::module_query`] and [`Context::module_unload`] wrappers.  See the SDK
//! `docs/module-loader.md`.

#![no_std]

pub use module_abi::{
    ModuleArena, ModuleContext, ModuleEntry, ModuleExport, ModuleExportBinding, ModuleHandle,
    ModuleInfo, ModuleLoad, ModuleVtable, Services, ABI_VERSION, MODULE_ARENA_EXECUTABLE,
    MODULE_BASE, MODULE_CONTEXT_VERSION, MODULE_ENTRY_LEGACY, MODULE_ERR_ARGUMENT, MODULE_ERR_BUSY,
    MODULE_ERR_DIGEST, MODULE_ERR_LIMIT, MODULE_ERR_NOT_FOUND, MODULE_ERR_REGION, MODULE_ERR_SIZE,
    MODULE_ERR_STATE, MODULE_ERR_TRANSPORT, MODULE_EXPORT_MAX, MODULE_INTERFACE_MAX,
    MODULE_LOAD_REPLACE, MODULE_MAX, MODULE_NAME_MAX, MODULE_OFFSET_ALIGN, MODULE_OK,
    MODULE_REGION_MAX, MODULE_STATE_ENTERED, MODULE_STATE_LOADED, MODULE_URL_MAX,
    MODULE_VERSION_MAX,
};

/// One export in a [`ModuleLoadRequest`].
///
/// `interface` is the UTF-8 interface name, `version` its version and `offset`
/// the image-relative address of the interface's C function table (zero for an
/// interface-only declaration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleExportSpec<'a> {
    /// Interface name bytes.
    pub interface: &'a [u8],
    /// Interface version.
    pub version: u16,
    /// Image-relative table offset; zero means no table.
    pub offset: u32,
}

/// A verified-image load request for [`Context::module_load`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleLoadRequest<'a> {
    /// Immutable image URL.
    pub url: &'a [u8],
    /// Expected SHA-256 of the complete fetched file.
    pub sha256: [u8; 32],
    /// Exact module name.
    pub name: &'a [u8],
    /// Exact module version.
    pub version: &'a [u8],
    /// Export declarations.
    pub exports: &'a [ModuleExportSpec<'a>],
    /// Arena region index.
    pub region: u32,
    /// [`MODULE_LOAD_REPLACE`] or zero.
    pub flags: u32,
    /// Maximum accepted image bytes; zero means the region size.
    pub max_size: u32,
}

impl<'a> ModuleLoadRequest<'a> {
    /// A request with no exports and default flags.
    pub const fn new(url: &'a [u8], sha256: [u8; 32], name: &'a [u8], version: &'a [u8]) -> Self {
        Self {
            url,
            sha256,
            name,
            version,
            exports: &[],
            region: 0,
            flags: 0,
            max_size: 0,
        }
    }
}

const fn aligned_offset(offset: u32) -> bool {
    offset.is_multiple_of(MODULE_OFFSET_ALIGN)
}

fn bounded_text(value: &[u8], max: u32) -> bool {
    !value.is_empty() && value.len() <= max as usize && core::str::from_utf8(value).is_ok()
}

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

    /// Read one 512-byte SCSI sector from `target` at `lba`.
    pub fn block_read(&self, target: u8, lba: u32, out: &mut [u8; 512]) -> bool {
        // SAFETY: the kernel writes exactly 512 bytes into the module buffer.
        unsafe { (self.services.block_read)(target, lba, out.as_mut_ptr()) == 0 }
    }

    /// Write one 512-byte SCSI sector through the ABI version 11 service.
    pub fn block_write(&self, target: u8, lba: u32, data: &[u8; 512]) -> bool {
        // SAFETY: the kernel reads exactly 512 bytes from the module buffer.
        unsafe { (self.services.block_write)(target, lba, data.as_ptr()) == 0 }
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

    /// Query the bounded module arena.
    ///
    /// Returns [`MODULE_ERR_ARGUMENT`] if the kernel rejects the descriptor.
    pub fn module_arena(&self) -> Result<ModuleArena, i32> {
        let mut arena = ModuleArena::EMPTY;
        let status = unsafe { (self.services.module_arena)(&mut arena) };
        if status == MODULE_OK {
            Ok(arena)
        } else {
            Err(status)
        }
    }

    /// Fetch, verify and copy one module image into a chosen arena region.
    ///
    /// The kernel performs the fetch, the SHA-256 check and the copy; it
    /// returns the handle only when the verified image is fully installed and
    /// visible to [`Context::module_find`].  The URL, name, version and export
    /// interfaces are copied before this call returns.
    pub fn module_load(&self, request: &ModuleLoadRequest<'_>) -> Result<ModuleHandle, i32> {
        if !bounded_text(request.url, MODULE_URL_MAX)
            || !bounded_text(request.name, MODULE_NAME_MAX)
            || !bounded_text(request.version, MODULE_VERSION_MAX)
        {
            return Err(MODULE_ERR_ARGUMENT);
        }
        if request.exports.len() > MODULE_EXPORT_MAX as usize {
            return Err(MODULE_ERR_LIMIT);
        }
        for export in request.exports {
            if !bounded_text(export.interface, MODULE_INTERFACE_MAX)
                || !aligned_offset(export.offset)
            {
                return Err(MODULE_ERR_ARGUMENT);
            }
        }

        let mut exports = [ModuleExport::EMPTY; MODULE_EXPORT_MAX as usize];
        for (slot, spec) in exports.iter_mut().zip(request.exports.iter()) {
            *slot = ModuleExport {
                interface: spec.interface.as_ptr(),
                interface_len: spec.interface.len() as u32,
                version: spec.version,
                reserved: 0,
                offset: spec.offset,
            };
        }
        let load = ModuleLoad {
            url: request.url.as_ptr(),
            url_len: request.url.len() as u32,
            sha256: request.sha256,
            name: request.name.as_ptr(),
            name_len: request.name.len() as u32,
            version: request.version.as_ptr(),
            version_len: request.version.len() as u32,
            exports: exports.as_ptr(),
            export_count: request.exports.len() as u32,
            region: request.region,
            flags: request.flags,
            max_size: request.max_size,
            reserved: 0,
        };
        let mut handle: ModuleHandle = 0;
        let status = unsafe { (self.services.module_load)(&load, &mut handle) };
        if status == MODULE_OK {
            Ok(handle)
        } else {
            Err(status)
        }
    }

    /// Enter a loaded image at `entry_offset` with an optional manager context.
    ///
    /// Returns the module's own vtable status (0 for a null vtable).  A loader
    /// failure is returned as `Err(MODULE_ERR_*)`; the entry is not called in
    /// that case.  The kernel performs the data-cache writeback and
    /// instruction-cache invalidate before entry.
    ///
    /// # Safety
    /// `context` must be null or point to a valid [`ModuleContext`] (or a
    /// manager type whose leading bytes are a valid `ModuleContext`) for the
    /// duration of the call and, if the module retains it, while it is loaded.
    pub unsafe fn module_execute(
        &self,
        handle: ModuleHandle,
        entry_offset: u32,
        context: *const ModuleContext,
    ) -> Result<i32, i32> {
        if !aligned_offset(entry_offset) {
            return Err(MODULE_ERR_ARGUMENT);
        }
        let mut module_status = 0i32;
        let status = unsafe {
            (self.services.module_execute)(handle, entry_offset, context, &mut module_status)
        };
        if status == MODULE_OK {
            Ok(module_status)
        } else {
            Err(status)
        }
    }

    /// Find a loaded module by exact name and version.
    pub fn module_find(&self, name: &[u8], version: &[u8]) -> Result<ModuleHandle, i32> {
        if !bounded_text(name, MODULE_NAME_MAX) || !bounded_text(version, MODULE_VERSION_MAX) {
            return Err(MODULE_ERR_ARGUMENT);
        }
        let mut handle: ModuleHandle = 0;
        let status = unsafe {
            (self.services.module_find)(
                name.as_ptr(),
                name.len() as u32,
                version.as_ptr(),
                version.len() as u32,
                &mut handle,
            )
        };
        if status == MODULE_OK {
            Ok(handle)
        } else {
            Err(status)
        }
    }

    /// Find an export by exact interface and version.
    ///
    /// The binding carries the live handle and the absolute address of the
    /// interface-defined C table, or a null table for an interface-only
    /// declaration.
    pub fn module_find_export(
        &self,
        interface: &[u8],
        version: u16,
    ) -> Result<ModuleExportBinding, i32> {
        if !bounded_text(interface, MODULE_INTERFACE_MAX) {
            return Err(MODULE_ERR_ARGUMENT);
        }
        let mut binding = ModuleExportBinding::EMPTY;
        let status = unsafe {
            (self.services.module_find_export)(
                interface.as_ptr(),
                interface.len() as u32,
                version,
                &mut binding,
            )
        };
        if status == MODULE_OK {
            Ok(binding)
        } else {
            Err(status)
        }
    }

    /// Query a loaded handle's region, base, size and state.
    pub fn module_query(&self, handle: ModuleHandle) -> Result<ModuleInfo, i32> {
        let mut info = ModuleInfo::EMPTY;
        let status = unsafe { (self.services.module_query)(handle, &mut info) };
        if status == MODULE_OK {
            Ok(info)
        } else {
            Err(status)
        }
    }

    /// Unload a handle and release its region.
    ///
    /// The manager must ensure no dependent still holds the handle or an
    /// export table.  A stale or already-unloaded handle fails with
    /// [`MODULE_ERR_STATE`].
    pub fn module_unload(&self, handle: ModuleHandle) -> Result<(), i32> {
        let status = unsafe { (self.services.module_unload)(handle) };
        if status == MODULE_OK {
            Ok(())
        } else {
            Err(status)
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

/// Define a manager-aware kernel entry point.
///
/// The supplied function must have the signature
/// `fn(&Context, *const ModuleContext) -> *const ModuleVtable`.
///
/// This defines `_start` at image offset 0, so the manifest's `entry_offset`
/// is 0.  A module that needs a different manager entry uses its own linker
/// script and records that offset in the manifest.  Legacy modules built with
/// [`entry!`] ignore the extra context argument and keep working.
#[macro_export]
macro_rules! entry_with_context {
    ($run:path) => {
        #[no_mangle]
        pub unsafe extern "C" fn _start(
            services: *const $crate::Services,
            manager: *const $crate::ModuleContext,
        ) -> *const $crate::ModuleVtable {
            match $crate::Context::new(services) {
                Some(context) => $run(&context, manager),
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    static ARENA_CALLS: AtomicU32 = AtomicU32::new(0);

    const DIGEST: [u8; 32] = [0x11; 32];
    const HANDLE: ModuleHandle = 0x0000_0001_0000_0002;

    extern "C" fn write_stub(_: *const u8, _: usize) {}
    extern "C" fn clock_stub() -> u64 {
        0
    }
    unsafe extern "C" fn alloc_stub(_: usize, _: usize) -> *mut u8 {
        core::ptr::null_mut()
    }
    extern "C" fn noop() {}
    extern "C" fn noop_u32(_: u32) {}
    unsafe extern "C" fn tcp_listen_stub(_: u32, _: u16) -> i32 {
        -1
    }
    extern "C" fn tcp_state_stub(_: u32) -> u32 {
        0
    }
    unsafe extern "C" fn tcp_recv_stub(_: u32, _: *mut u8, _: usize) -> isize {
        -1
    }
    unsafe extern "C" fn tcp_send_stub(_: u32, _: *const u8, _: usize) -> isize {
        -1
    }
    unsafe extern "C" fn console_read_stub(_: u64, _: *mut u8, _: usize) -> isize {
        -1
    }
    extern "C" fn console_total_stub() -> u64 {
        0
    }
    unsafe extern "C" fn status_stub(_: *mut u8, _: usize) -> isize {
        -1
    }
    unsafe extern "C" fn config_get_stub(_: *const u8, _: usize, _: *mut u8, _: usize) -> isize {
        -1
    }
    unsafe extern "C" fn info_stub(_: u32, _: *mut u8, _: usize) -> isize {
        -1
    }
    unsafe extern "C" fn screen_read_stub(_: u32, _: *mut u8, _: usize) -> isize {
        -1
    }
    unsafe extern "C" fn block_read_stub(_: u8, _: u32, _: *mut u8) -> i32 {
        -1
    }
    unsafe extern "C" fn block_write_stub(_: u8, _: u32, _: *const u8) -> i32 {
        -1
    }

    unsafe extern "C" fn arena_stub(out: *mut ModuleArena) -> i32 {
        if out.is_null() {
            return MODULE_ERR_ARGUMENT;
        }
        ARENA_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            *out = ModuleArena {
                base: 0x8840_0000,
                length: 0x0020_0000,
                region_size: 0x0010_0000,
                region_count: 2,
                used_lo: 1,
                used_hi: 0,
                flags: MODULE_ARENA_EXECUTABLE,
                reserved: 0,
            };
        }
        MODULE_OK
    }

    unsafe extern "C" fn load_stub(request: *const ModuleLoad, out: *mut ModuleHandle) -> i32 {
        if request.is_null() || out.is_null() {
            return MODULE_ERR_ARGUMENT;
        }
        let request = unsafe { &*request };
        if request.url.is_null()
            || request.name.is_null()
            || request.version.is_null()
            || (request.export_count > 0 && request.exports.is_null())
        {
            return MODULE_ERR_ARGUMENT;
        }
        if request.sha256 == [0u8; 32] {
            return MODULE_ERR_DIGEST;
        }
        unsafe {
            *out = HANDLE;
        }
        MODULE_OK
    }

    unsafe extern "C" fn execute_stub(
        handle: ModuleHandle,
        entry_offset: u32,
        _context: *const ModuleContext,
        out_status: *mut i32,
    ) -> i32 {
        if handle == 0 || !entry_offset.is_multiple_of(MODULE_OFFSET_ALIGN) {
            return MODULE_ERR_STATE;
        }
        if !out_status.is_null() {
            unsafe {
                *out_status = 7;
            }
        }
        MODULE_OK
    }

    unsafe extern "C" fn find_stub(
        name: *const u8,
        name_len: u32,
        version: *const u8,
        version_len: u32,
        out: *mut ModuleHandle,
    ) -> i32 {
        if name.is_null() || version.is_null() || out.is_null() || name_len == 0 || version_len == 0
        {
            return MODULE_ERR_ARGUMENT;
        }
        unsafe {
            *out = HANDLE;
        }
        MODULE_OK
    }

    unsafe extern "C" fn find_export_stub(
        interface: *const u8,
        interface_len: u32,
        version: u16,
        out: *mut ModuleExportBinding,
    ) -> i32 {
        if interface.is_null() || out.is_null() || interface_len == 0 {
            return MODULE_ERR_ARGUMENT;
        }
        unsafe {
            *out = ModuleExportBinding {
                handle: HANDLE,
                table: 0x8840_0100 as *const u8,
                version,
                reserved: 0,
            };
        }
        MODULE_OK
    }

    unsafe extern "C" fn query_stub(handle: ModuleHandle, out: *mut ModuleInfo) -> i32 {
        if out.is_null() || handle == 0 {
            return MODULE_ERR_ARGUMENT;
        }
        unsafe {
            *out = ModuleInfo {
                handle,
                base: 0x8850_0000,
                image_size: 0x1000,
                region: 1,
                export_count: 1,
                state: MODULE_STATE_ENTERED,
                reserved: 0,
            };
        }
        MODULE_OK
    }

    unsafe extern "C" fn unload_stub(_: ModuleHandle) -> i32 {
        MODULE_ERR_STATE
    }

    static FAKE: Services = Services {
        version: ABI_VERSION,
        write: write_stub,
        clock_ms: clock_stub,
        alloc: alloc_stub,
        poll: noop,
        tcp_listen: tcp_listen_stub,
        tcp_state: tcp_state_stub,
        tcp_recv: tcp_recv_stub,
        tcp_send: tcp_send_stub,
        tcp_close: noop_u32,
        tcp_abort: noop_u32,
        console_read: console_read_stub,
        console_total: console_total_stub,
        reset: noop,
        poweroff: noop,
        status: status_stub,
        config_get: config_get_stub,
        info: info_stub,
        screen_read: screen_read_stub,
        block_read: block_read_stub,
        block_write: block_write_stub,
        module_arena: arena_stub,
        module_load: load_stub,
        module_execute: execute_stub,
        module_find: find_stub,
        module_find_export: find_export_stub,
        module_query: query_stub,
        module_unload: unload_stub,
    };

    fn context() -> Context {
        unsafe { Context::new(&FAKE as *const Services) }.expect("fake table")
    }

    #[test]
    fn arena_query_returns_the_reported_range() {
        let before = ARENA_CALLS.load(Ordering::Relaxed);
        let arena = context().module_arena().expect("arena");
        assert_eq!(arena.base, 0x8840_0000);
        assert_eq!(arena.length, 0x0020_0000);
        assert_eq!(arena.region_count, 2);
        assert_eq!(arena.used_lo, 1);
        assert_eq!(arena.flags, MODULE_ARENA_EXECUTABLE);
        assert_eq!(ARENA_CALLS.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn load_builds_a_bounded_request() {
        let exports = [ModuleExportSpec {
            interface: b"irix7.test",
            version: 1,
            offset: 0x20,
        }];
        let request = ModuleLoadRequest {
            url: b"https://example.invalid/m.module",
            sha256: DIGEST,
            name: b"m",
            version: b"1.0.0",
            exports: &exports,
            region: 1,
            flags: 0,
            max_size: 0,
        };
        assert_eq!(context().module_load(&request), Ok(HANDLE));
    }

    #[test]
    fn load_propagates_a_loader_error() {
        let request = ModuleLoadRequest::new(b"u", [0u8; 32], b"m", b"1");
        assert_eq!(context().module_load(&request), Err(MODULE_ERR_DIGEST));
    }

    #[test]
    fn load_rejects_bad_arguments_before_the_kernel() {
        let context = context();
        assert_eq!(
            context.module_load(&ModuleLoadRequest::new(b"", DIGEST, b"m", b"1")),
            Err(MODULE_ERR_ARGUMENT)
        );
        assert_eq!(
            context.module_load(&ModuleLoadRequest::new(b"u", DIGEST, b"", b"1")),
            Err(MODULE_ERR_ARGUMENT)
        );
        assert_eq!(
            context.module_load(&ModuleLoadRequest::new(b"u", DIGEST, b"m", b"")),
            Err(MODULE_ERR_ARGUMENT)
        );
        assert_eq!(
            context.module_load(&ModuleLoadRequest::new(b"u", DIGEST, b"\xff", b"1")),
            Err(MODULE_ERR_ARGUMENT)
        );

        let unaligned = [ModuleExportSpec {
            interface: b"i",
            version: 1,
            offset: 1,
        }];
        let mut request = ModuleLoadRequest::new(b"u", DIGEST, b"m", b"1");
        request.exports = &unaligned;
        assert_eq!(context.module_load(&request), Err(MODULE_ERR_ARGUMENT));

        let many = [ModuleExportSpec {
            interface: b"i",
            version: 1,
            offset: 0,
        }; 17];
        let mut request = ModuleLoadRequest::new(b"u", DIGEST, b"m", b"1");
        request.exports = &many;
        assert_eq!(context.module_load(&request), Err(MODULE_ERR_LIMIT));
    }

    #[test]
    fn execute_returns_the_module_status_separately() {
        let context = context();
        assert_eq!(
            unsafe { context.module_execute(HANDLE, 0, core::ptr::null()) },
            Ok(7)
        );
        assert_eq!(
            unsafe { context.module_execute(HANDLE, 2, core::ptr::null()) },
            Err(MODULE_ERR_ARGUMENT)
        );
    }

    #[test]
    fn find_and_find_export_validate_and_return_bindings() {
        let context = context();
        assert_eq!(context.module_find(b"m", b"1.0.0"), Ok(HANDLE));
        assert_eq!(context.module_find(b"", b"1.0.0"), Err(MODULE_ERR_ARGUMENT));

        let binding = context
            .module_find_export(b"irix7.test", 1)
            .expect("binding");
        assert_eq!(binding.handle, HANDLE);
        assert_eq!(binding.table, 0x8840_0100 as *const u8);
        assert_eq!(binding.version, 1);
        assert_eq!(context.module_find_export(b"", 1), Err(MODULE_ERR_ARGUMENT));
    }

    #[test]
    fn query_and_unload_map_status_codes() {
        let context = context();
        let info = context.module_query(HANDLE).expect("info");
        assert_eq!(info.handle, HANDLE);
        assert_eq!(info.region, 1);
        assert_eq!(info.state, MODULE_STATE_ENTERED);
        assert_eq!(context.module_unload(HANDLE), Err(MODULE_ERR_STATE));
    }

    #[test]
    fn context_defaults_are_versioned() {
        assert_eq!(ModuleContext::EMPTY.version, MODULE_CONTEXT_VERSION);
        assert!(ModuleContext::EMPTY.manager.is_null());
    }
}
