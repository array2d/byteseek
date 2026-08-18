//! rwir —— 注册进 kvlang runtime 的一等公民。每个 rwir 一个子模块：
//!   llm    : llm.call(sid) -> kind        调 LLM，解析动作块，写 action/*
//!   shell  : shell.run(sid)               跑 bash
//!   python : python.run(sid)              跑 python3
//!   agent  : agent.spawn(sid)             派生子 session + 子 vthread
//!   io     : print / println / cerr       行输出（非 kvlang builtin，引擎自理）

pub mod agent;
pub mod io;
pub mod llm;
pub mod python;
pub mod shell;

use std::process::Command;

use crate::engine::{Engine, TOOL_CAP};
use crate::ffi::*;

/// rwir 注册表：(opcode, 读参数, 写参数, 签名)。
pub const REGS: &[(&str, i32, i32, &str)] = &[
    ("llm.call", 1, 1, "any\nany"),
    ("shell.run", 1, 0, "any"),
    ("python.run", 1, 0, "any"),
    ("agent.spawn", 1, 0, "any"),
    ("print", 1, 0, "any..."),
    ("println", 1, 0, "any..."),
    ("cerr", 1, 0, "any..."),
];

pub fn register(eng: &Engine) {
    for (op, nr, nw, sig) in REGS {
        unsafe { rwext_register(eng.conn_ptr(), cs(op).as_ptr(), *nr, *nw, cs(sig).as_ptr()) };
    }
}

/// 主导驱动循环遇到 rwir 就分派到对应子模块。
pub fn dispatch(eng: &Engine, op: &str, pc: &str) {
    match op {
        "print" | "println" | "cerr" => io::print_line(eng, pc),
        "llm.call" => {
            let sid = eng.read0(pc);
            let kind = llm::call(eng, &sid);
            eng.set_kv(&eng.write0(pc), &kind);
        }
        "shell.run" => shell::run(eng, &eng.read0(pc)),
        "python.run" => python::run(eng, &eng.read0(pc)),
        "agent.spawn" => agent::spawn(eng, &eng.read0(pc)),
        other => eprintln!("[byteseek] 未知 rwir: {other} @ {pc}"),
    }
}

/// shell.run / python.run 共用：跑 `action/arg` 里的代码，输出截断后 append 进 msg。
pub(crate) fn tool_run(eng: &Engine, sid: &str, label: &str, prog: &str, arg_flag: &str) {
    let code = eng.get_kv(&format!("/session/{sid}/action/arg"));
    println!("\n🔧 [{sid}] {label}:\n{code}");
    let out = Command::new(prog).arg(arg_flag).arg(&code).output();
    let mut result = match out {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            let e = String::from_utf8_lossy(&o.stderr);
            if !e.trim().is_empty() {
                s.push_str("\n[stderr]\n");
                s.push_str(&e);
            }
            if s.trim().is_empty() {
                s = format!("(无输出, exit={})", o.status.code().unwrap_or(-1));
            }
            s
        }
        Err(e) => format!("执行失败: {e}"),
    };
    if result.len() > TOOL_CAP {
        result.truncate(TOOL_CAP);
        result.push_str("\n…(已截断)");
    }
    println!("↳ 输出:\n{result}");
    eng.append_msg(sid, "user", &format!("{label} 输出:\n{result}"));
}
