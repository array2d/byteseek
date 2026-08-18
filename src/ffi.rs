//! C ABI：layout（Rust cdylib，durable 后端）+ runtime（模式2 主导执行）+ rwext。

use std::ffi::{c_char, c_int, c_void, CStr, CString};

#[repr(C)]
pub struct RwextConn {
    pub kv: *mut c_void,
}

unsafe extern "C" {
    pub fn kvlang_layout_file(
        file: *const c_char,
        dsn: *const c_char,
        entry: *mut c_char,
        entry_cap: u32,
        err: *mut c_char,
        err_cap: u32,
    ) -> c_int;

    pub fn kvlang_rt_connect(dsn: *const c_char) -> *mut c_void;
    pub fn kvlang_rt_disconnect(rt: *mut c_void);
    pub fn kvlang_rt_kv(rt: *mut c_void) -> *mut c_void;
    pub fn kvlang_rt_bootstrap(
        rt: *mut c_void,
        funcname: *const c_char,
        args: *const *const c_char,
        nargs: c_int,
    ) -> *mut c_char;
    pub fn kvlang_rt_execute_vthread(
        rt: *mut c_void,
        vid: *const c_char,
        out_pc: *mut *mut c_char,
    ) -> c_int;

    pub fn rwext_register(
        c: *mut RwextConn,
        opcode: *const c_char,
        nr: c_int,
        nw: c_int,
        sig: *const c_char,
    ) -> c_int;
    pub fn rwext_set(c: *mut RwextConn, key: *const c_char, val: *const c_char) -> c_int;
    pub fn rwext_get(c: *mut RwextConn, key: *const c_char) -> *mut c_char;
    pub fn rwext_mkindex(c: *mut RwextConn, path: *const c_char) -> c_int;
    pub fn rwext_params(c: *mut RwextConn, pc: *const c_char) -> *mut c_char;
    pub fn rwext_resolve_read(c: *mut RwextConn, pc: *const c_char, idx: c_int) -> *mut c_char;
    pub fn rwext_resolve_write(c: *mut RwextConn, pc: *const c_char, idx: c_int) -> *mut c_char;
    pub fn rwext_print_line(
        c: *mut RwextConn,
        pc: *const c_char,
        rawnl: *mut c_int,
        cerr: *mut c_int,
    ) -> *mut c_char;
    pub fn rwext_next_pc(pc: *const c_char) -> *mut c_char;
}

pub fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// 接管 C 侧 malloc 的字符串（读出后 free）。
pub fn take(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { libc::free(p as *mut c_void) };
    s
}
