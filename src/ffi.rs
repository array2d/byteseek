//! C ABI —— 直连 stock 三方 .so（camelCase，符合 deepx-design/doc/abi-naming-standard.md）：
//!   kvspace-durable : 地址空间 + KV 存取 + TLV 编解码（byteseek 自持 kvspace 句柄）
//!   kvlang runtime  : 模式2 主导执行 + rwirext 宿主 ABI
//!   kvlang layout   : .kv 编译入库（文件 / 内存源码 / 只校验）
//! 不再经自建 snake_case shim（已删，p5 不向后兼容）。

use std::ffi::{c_char, c_int, c_void, CStr, CString};

/// kvspaceDecodeHead 输出（与 kvspace-durable/src/ffi.rs::kvspaceHead_t 对齐）。
#[repr(C)]
#[derive(Default)]
pub struct KvspaceHead {
    pub kind: [u8; 32],
    pub is_ptr: u8,
    pub array_len: i32,
    pub body_len: i32,
    pub body_offset: i32,
    pub ndim: i32,
    pub dims: [i32; 8],
}

unsafe extern "C" {
    // ── kvspace-durable：KV 存取 + TLV ────────────────────────────────
    pub fn kvspaceConnect(dsn: *const c_char) -> *mut c_void;
    pub fn kvspaceFree(h: *mut c_void);
    pub fn kvspaceClear(h: *mut c_void, err: *mut c_char, err_cap: u32) -> c_int;
    pub fn kvspaceBytesFree(p: *mut u8, len: u32);
    pub fn kvspaceSet(
        h: *mut c_void,
        keys: *const *const c_char,
        vals: *const u8,
        lens: *const u32,
        n: u32,
        err: *mut c_char,
        err_cap: u32,
    ) -> c_int;
    pub fn kvspaceGet(
        h: *mut c_void,
        key: *const c_char,
        out: *mut *mut u8,
        out_len: *mut u32,
    ) -> c_int;
    pub fn kvspaceMkindex(
        h: *mut c_void,
        path: *const c_char,
        err: *mut c_char,
        err_cap: u32,
    ) -> c_int;
    pub fn kvspaceNewCharByte(
        bytes: *const u8,
        len: u32,
        out: *mut *mut u8,
        out_len: *mut u32,
    ) -> c_int;
    pub fn kvspaceDecodeHead(data: *const u8, data_len: u32, out: *mut KvspaceHead) -> c_int;

    // ── kvlang runtime：模式2 执行 ───────────────────────────────────
    pub fn kvlangRuntimeConnect(dsn: *const c_char) -> *mut c_void;
    pub fn kvlangRuntimeDisconnect(rt: *mut c_void);
    pub fn kvlangRuntimeBootstrap(
        rt: *mut c_void,
        funcname: *const c_char,
        args: *const *const c_char,
        nargs: c_int,
    ) -> *mut c_char;
    pub fn kvlangRuntimeExecuteVthread(
        rt: *mut c_void,
        vid: *const c_char,
        out_pc: *mut *mut c_char,
    ) -> c_int;

    // ── kvlang runtime：rwirext 宿主 ABI（均传 kvspace 句柄）─────────
    pub fn kvlang_rwirextRegister(
        kvspace: *mut c_void,
        opcode: *const c_char,
        nr: c_int,
        nw: c_int,
        sig: *const c_char,
    ) -> c_int;
    pub fn kvlang_rwirextParams(kvspace: *mut c_void, pc: *const c_char) -> *mut c_char;
    pub fn kvlang_rwirextResolveRead(
        kvspace: *mut c_void,
        pc: *const c_char,
        idx: c_int,
    ) -> *mut c_char;
    pub fn kvlang_rwirextResolveWrite(
        kvspace: *mut c_void,
        pc: *const c_char,
        idx: c_int,
    ) -> *mut c_char;
    pub fn kvlang_rwirextPrintLine(
        kvspace: *mut c_void,
        pc: *const c_char,
        rawnl: *mut c_int,
        cerr: *mut c_int,
    ) -> *mut c_char;
    pub fn kvlang_rwirextNextPc(pc: *const c_char) -> *mut c_char;

    // ── kvlang layout：.kv 编译入库 ──────────────────────────────────
    pub fn kvlangLayoutFile(
        path: *const c_char,
        dsn: *const c_char,
        entry: *mut c_char,
        entry_cap: u32,
        err: *mut c_char,
        err_cap: u32,
    ) -> c_int;
    pub fn kvlangLayoutSrc(
        src: *const c_char,
        dsn: *const c_char,
        entry: *mut c_char,
        entry_cap: u32,
        err: *mut c_char,
        err_cap: u32,
    ) -> c_int;
    pub fn kvlangLayoutVet(src: *const c_char, err: *mut c_char, err_cap: u32) -> c_int;
}

pub fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// 接管 C runtime（libc malloc）的字符串（读出后 libc::free）。
/// 仅用于 kvlang runtime / rwirext 的返回值——kvspace（Rust）缓冲区必须走 kvspaceBytesFree。
pub fn take(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { libc::free(p as *mut c_void) };
    s
}

/// 从定长缓冲区读 NUL 终止字符串（layout 的 entry/err 输出）。
pub fn cbuf(buf: &[u8]) -> String {
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}
