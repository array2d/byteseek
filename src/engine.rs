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
    pub rt: *mut c_void,
    pub conn: RwextConn,
    pub max_steps: u32, // 每个 session 的最大 LLM 轮数（BYTESEEK_MAX_STEPS）
    pub subs: Cell<u32>,
}

impl Engine {
    pub fn conn_ptr(&self) -> *mut RwextConn {
        &self.conn as *const RwextConn as *mut RwextConn
    }

    // ── kvspace 读写（绝对路径，走 runtime 的 durable/redis）────────────
    pub fn set_kv(&self, key: &str, val: &str) {
        unsafe { rwext_set(self.conn_ptr(), cs(key).as_ptr(), cs(val).as_ptr()) };
    }
    pub fn get_kv(&self, key: &str) -> String {
        take(unsafe { rwext_get(self.conn_ptr(), cs(key).as_ptr()) })
    }
    pub fn mkindex(&self, path: &str) {
        unsafe { rwext_mkindex(self.conn_ptr(), cs(path).as_ptr()) };
    }

    // rwir 派发时按下标解析读/写槽
    pub fn read0(&self, pc: &str) -> String {
        take(unsafe { rwext_resolve_read(self.conn_ptr(), cs(pc).as_ptr(), 0) })
    }
    pub fn write0(&self, pc: &str) -> String {
        take(unsafe { rwext_resolve_write(self.conn_ptr(), cs(pc).as_ptr(), 0) })
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
        let vid = take(unsafe { kvlang_rt_bootstrap(self.rt, fnc.as_ptr(), null_mut(), 0) });
        if vid.is_empty() {
            eprintln!("[byteseek] bootstrap init 失败");
            return;
        }
        let vpc = format!("/vthread/{vid}/\u{2025}pc");
        loop {
            let mut pc: *mut c_char = null_mut();
            let rc = unsafe { kvlang_rt_execute_vthread(self.rt, cs(&vid).as_ptr(), &mut pc) };
            if rc == 0 {
                break; // done
            }
            if rc != 1 {
                eprintln!("[byteseek] execute_vthread 错误 rc={rc}");
                break;
            }
            let c = take(pc);
            let params = take(unsafe { rwext_params(self.conn_ptr(), cs(&c).as_ptr()) });
            let op = params.lines().next().unwrap_or("").to_string();
            rwir::dispatch(self, &op, &c);
            let nxt = take(unsafe { rwext_next_pc(cs(&c).as_ptr()) });
            self.set_kv(&vpc, &nxt);
        }
    }
}
