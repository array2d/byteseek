//! Engine —— byteseek 主脑运行时核心：kvspace(redis) 读写、talk 队列、主导驱动 vthread。
//! 具体 rwir（llm/shell/python/io/kvlayout）分组在 `crate::rwir`。

use std::ffi::{c_char, c_void};
use std::ptr::null_mut;

use crate::ffi::*;
use crate::rwir;

pub const TOOL_CAP: usize = 4000; // shell/python 工具输出截断

pub struct Engine {
    pub rt: *mut c_void, // kvlang runtime 句柄
    pub kv: *mut c_void, // kvspace 句柄（byteseek 自持，同时传给 rwirext）
    pub dsn: String,     // kvspace DSN（layout rwir 需要）
}

impl Engine {
    // ── kvspace 读写（绝对路径，char/utf8 TLV 编解码）───────────────────
    pub fn set_kv(&self, key: &str, val: &str) {
        unsafe {
            let utf32: Vec<u32> = val.chars().map(|c| c as u32).collect();
            let raw: Vec<u8> = utf32.iter().flat_map(|v| v.to_le_bytes()).collect();
            let dims = [utf32.len() as i32];
            let (mut buf, mut len) = (null_mut(), 0u32);
            kvspaceTlvEncode(
                cs("char/utf32").as_ptr(),
                raw.as_ptr(),
                raw.len() as u32,
                dims.as_ptr(),
                1,
                &mut buf,
                &mut len,
            );
            let ck = cs(key);
            let keys = [ck.as_ptr()];
            let lens = [len];
            let mut err = [0u8; 256];
            kvspaceSet(
                self.kv,
                keys.as_ptr(),
                buf,
                lens.as_ptr(),
                1,
                err.as_mut_ptr() as *mut c_char,
                256,
            );
            kvspaceBytesFree(buf, len);
        }
    }
    pub fn get_kv(&self, key: &str) -> String {
        unsafe {
            let (mut out, mut olen) = (null_mut(), 0u32);
            kvspaceGet(self.kv, cs(key).as_ptr(), &mut out, &mut olen);
            if out.is_null() || olen == 0 {
                return String::new();
            }
            let mut head = KvspaceHead::default();
            kvspaceDecodeHead(out, olen, &mut head);
            let kx = String::from_utf8_lossy(&head.kindexpr)
                .trim_end_matches('\0')
                .to_string();
            let (_, _, kind) = parse_kindexpr(&kx);
            let (bo, bl) = (head.body_offset as usize, head.body_len.max(0) as usize);
            let s = if kind == "char/utf32" {
                std::slice::from_raw_parts(out.add(bo), bl)
                    .chunks_exact(4)
                    .map(|c| {
                        char::from_u32(u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .unwrap_or('\u{FFFD}')
                    })
                    .collect()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(out.add(bo), bl)).into_owned()
            };
            kvspaceBytesFree(out, olen);
            s
        }
    }
    // rwir 派发时按下标解析读/写槽（rwirext 宿主 ABI，传 kvspace 句柄）
    pub fn read0(&self, pc: &str) -> String {
        take(unsafe { kvlang_rwirextResolveRead(self.kv, cs(pc).as_ptr(), 0) })
    }
    pub fn write0(&self, pc: &str) -> String {
        take(unsafe { kvlang_rwirextResolveWrite(self.kv, cs(pc).as_ptr(), 0) })
    }

    // ── kvspace 结构操作（json/http/os 扩展遍历子树用）──────────────────
    pub fn list_kv(&self, prefix: &str) -> Vec<String> {
        unsafe {
            let (mut out, mut olen) = (null_mut(), 0u32);
            kvspaceList(self.kv, cs(prefix).as_ptr(), 0, 0, &mut out, &mut olen);
            if out.is_null() || olen == 0 {
                return Vec::new();
            }
            let s =
                String::from_utf8_lossy(std::slice::from_raw_parts(out, olen as usize)).into_owned();
            kvspaceBytesFree(out, olen);
            s.split('\n').filter(|x| !x.is_empty()).map(str::to_string).collect()
        }
    }
    pub fn del_tree(&self, prefix: &str) {
        unsafe {
            let mut err = [0u8; 256];
            kvspaceDelTree(self.kv, cs(prefix).as_ptr(), err.as_mut_ptr() as *mut c_char, 256);
        }
    }
    pub fn get_tlv(&self, key: &str) -> Vec<u8> {
        unsafe {
            let (mut out, mut olen) = (null_mut(), 0u32);
            kvspaceGet(self.kv, cs(key).as_ptr(), &mut out, &mut olen);
            if out.is_null() || olen == 0 {
                return Vec::new();
            }
            let v = std::slice::from_raw_parts(out, olen as usize).to_vec();
            kvspaceBytesFree(out, olen);
            v
        }
    }
    pub fn set_tlv(&self, key: &str, tlv: &[u8]) {
        if tlv.is_empty() {
            return;
        }
        unsafe {
            let ck = cs(key);
            let keys = [ck.as_ptr()];
            let lens = [tlv.len() as u32];
            let mut err = [0u8; 256];
            kvspaceSet(
                self.kv,
                keys.as_ptr(),
                tlv.as_ptr(),
                lens.as_ptr(),
                1,
                err.as_mut_ptr() as *mut c_char,
                256,
            );
        }
    }

    // ── talk 队列：byteseek session 下的对话记录（input 落入此处，可寻址/持久）──
    pub fn talk_push(&self, role: &str, content: &str) {
        let n: u32 = self
            .get_kv("/byteseek/session/talk/count")
            .trim()
            .parse()
            .unwrap_or(0);
        self.set_kv(&format!("/byteseek/session/talk/{n}/role"), role);
        self.set_kv(&format!("/byteseek/session/talk/{n}/content"), content);
        self.set_kv("/byteseek/session/talk/count", &(n + 1).to_string());
    }

    pub fn register(&self) {
        rwir::register(self);
    }

    // ── bootstrap 一个函数并主导驱动其 vthread 直到结束（模式2）──────────
    // funcname 按最后一个点切分为 pkg/name（如 "byteseek.main"、"byteseek/session/x.init"）。
    // 遇 ext rwir 就地 dispatch，再写回下一 pc。可重入：byteseek.run 在 dispatch 里嵌套调用本函数。
    pub fn run_fn(&self, funcname: &str) {
        let fnc = cs(funcname);
        let vid = take(unsafe { kvlangRuntimeBootstrap(self.rt, fnc.as_ptr(), null_mut(), 0) });
        if vid.is_empty() {
            eprintln!("[byteseek] bootstrap {funcname} 失败");
            return;
        }
        let vpc = format!("/vthread/{vid}/\u{2025}pc");
        loop {
            let mut pc: *mut c_char = null_mut();
            let rc = unsafe { kvlangRuntimeExecuteVthread(self.rt, cs(&vid).as_ptr(), &mut pc) };
            if rc == 0 {
                break; // done
            }
            if rc != 1 {
                let st = self.get_kv(&format!("/vthread/{vid}/\u{2025}status"));
                let msg = self.get_kv(&format!("/vthread/{vid}/\u{2025}error/msg"));
                eprintln!("[byteseek] vthread {vid} 错误 rc={rc} {st}: {msg}");
                break;
            }
            let c = take(pc);
            let params = take(unsafe { kvlang_rwirextParams(self.kv, cs(&c).as_ptr()) });
            let op = params.lines().next().unwrap_or("").to_string();
            rwir::dispatch(self, &op, &c);
            let nxt = take(unsafe { kvlang_rwirextNextPc(cs(&c).as_ptr()) });
            self.set_kv(&vpc, &nxt);
        }
    }
}
