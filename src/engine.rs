//! Engine —— corebrain 的运行时核心：kvspace(redis) 读写、消息树、主导驱动 vthread。
//! 具体 rwir（llm/shell/python/agent/io）分组在 `crate::rwir`。

use std::cell::Cell;
use std::ffi::{c_char, c_void};
use std::ptr::null_mut;

use crate::ffi::*;
use crate::rwir;

pub const TOOL_CAP: usize = 4000; // 工具输出截断
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

pub struct Engine {
    pub rt: *mut c_void, // kvlang runtime 句柄
    pub kv: *mut c_void, // kvspace 句柄（byteseek 自持，同时传给 rwirext）
    pub dsn: String,     // kvspace DSN（layout rwir 需要）
    pub max_steps: u32,  // 每个 session 的最大 LLM 轮数（BYTESEEK_MAX_STEPS）
    pub subs: Cell<u32>,
}

impl Engine {
    // ── kvspace 读写（绝对路径，char/utf8 TLV 编解码）───────────────────
    pub fn set_kv(&self, key: &str, val: &str) {
        unsafe {
            let (mut buf, mut len) = (null_mut(), 0u32);
            kvspaceNewCharByte(val.as_ptr(), val.len() as u32, &mut buf, &mut len);
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
            let (bo, bl) = (head.body_offset as usize, head.body_len.max(0) as usize);
            let s =
                String::from_utf8_lossy(std::slice::from_raw_parts(out.add(bo), bl)).into_owned();
            kvspaceBytesFree(out, olen);
            s
        }
    }
    pub fn mkindex(&self, path: &str) {
        let mut err = [0u8; 256];
        unsafe {
            kvspaceMkindex(
                self.kv,
                cs(path).as_ptr(),
                err.as_mut_ptr() as *mut c_char,
                256,
            )
        };
    }

    // rwir 派发时按下标解析读/写槽（rwirext 宿主 ABI，传 kvspace 句柄）
    pub fn read0(&self, pc: &str) -> String {
        take(unsafe { kvlang_rwirextResolveRead(self.kv, cs(pc).as_ptr(), 0) })
    }
    pub fn write0(&self, pc: &str) -> String {
        take(unsafe { kvlang_rwirextResolveWrite(self.kv, cs(pc).as_ptr(), 0) })
    }

    // ── 消息树 ──────────────────────────────────────────────────────────
    pub fn msg_count(&self, sid: &str) -> u32 {
        self.get_kv(&format!("/session/{sid}/msg/count"))
            .trim()
            .parse()
            .unwrap_or(0)
    }
    pub fn append_msg(&self, sid: &str, role: &str, content: &str) {
        let n = self.msg_count(sid);
        self.set_kv(&format!("/session/{sid}/msg/{n}/role"), role);
        self.set_kv(&format!("/session/{sid}/msg/{n}/content"), content);
        self.set_kv(&format!("/session/{sid}/msg/count"), &(n + 1).to_string());
    }
    pub fn read_msgs(&self, sid: &str) -> Vec<(String, String)> {
        let n = self.msg_count(sid);
        (0..n)
            .map(|i| {
                (
                    self.get_kv(&format!("/session/{sid}/msg/{i}/role")),
                    self.get_kv(&format!("/session/{sid}/msg/{i}/content")),
                )
            })
            .collect()
    }

    // 系统提示从树里读（seed_prompts 已写入 /byteseek/prompt/system）。
    pub fn system_prompt(&self) -> String {
        self.get_kv("/byteseek/prompt/system")
    }

    pub fn seed_session(&self, sid: &str, task: &str) {
        self.set_kv(&format!("/session/{sid}/model"), DEFAULT_MODEL);
        self.set_kv(&format!("/session/{sid}/task"), task);
        self.set_kv(&format!("/session/{sid}/steps"), "0");
        self.set_kv(&format!("/session/{sid}/msg/count"), "0");
        self.append_msg(sid, "user", task);
    }

    pub fn set_action(&self, sid: &str, kind: &str, arg: &str) {
        self.set_kv(&format!("/session/{sid}/action/kind"), kind);
        self.set_kv(&format!("/session/{sid}/action/arg"), arg);
        if kind == "final" {
            self.set_kv(&format!("/session/{sid}/final"), arg);
        }
    }

    pub fn register(&self) {
        rwir::register(self);
    }

    // ── 主导驱动一个 session 直到 vthread 结束（模式2）──────────────────
    // bootstrap 顶层 init 帧（同 term，无参）。init 里的 `agentloop()` 会在嵌套调用帧
    // 执行，其 while scope 才能正确寻址。sid 经 /byteseek/cursid 传入。
    pub fn run_entry(&self, session_id: &str) {
        self.set_kv("/byteseek/cursid", session_id);
        let fnc = cs("init");
        let vid = take(unsafe { kvlangRuntimeBootstrap(self.rt, fnc.as_ptr(), null_mut(), 0) });
        if vid.is_empty() {
            eprintln!("[byteseek] bootstrap init 失败");
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
                eprintln!("[byteseek] execute_vthread 错误 rc={rc}");
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
