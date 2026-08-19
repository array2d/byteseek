//! rwir —— 注册进 kvlang runtime 的一等公民。每个 rwir 一个子模块：
//!   llm     : llm.call(userinput) -> entry   调 LLM 生成一段 kv 程序，layout 到
//!             /lib/byteseek/session/<名>，返回入口名
//!   io      : print / println / cerr / input  行输出 + 读 stdin（input 落入 talk 队列）
//!   kvlayout: kvlanglayout.vet/layout/src      自造 kv 代码入库（layout C ABI）
//! shell.run / python.run / byteseek.run 直接在 dispatch 里处理（无独立状态）。

pub mod http;
pub mod io;
pub mod json;
pub mod kvlayout;
pub mod llm;

use std::process::Command;

use crate::engine::{Engine, TOOL_CAP};
use crate::ffi::*;

/// rwir 注册表：(opcode, 读参数, 写参数, 签名)。
pub const REGS: &[(&str, i32, i32, &str)] = &[
    ("llm.call", 1, 1, "any\nany"),
    ("byteseek.run", 1, 0, "any"),
    ("shell.run", 1, 1, "any\nany"),
    ("python.run", 1, 1, "any\nany"),
    ("input", 1, 1, "any\nany"),
    ("print", 1, 0, "any..."),
    ("println", 1, 0, "any..."),
    ("cerr", 1, 0, "any..."),
    ("json.to", 1, 1, "any\nany"),
    ("json.from", 1, 1, "any\nany"),
    ("http.call", 4, 1, "[]char/utf32\n[]char/utf32\n[]char/utf32\n[]char/utf32\n[]char/utf32"),
    ("kvlanglayout.vet", 1, 1, "any\nany"),
    ("kvlanglayout.layout", 1, 1, "any\nany"),
    ("kvlanglayout.src", 1, 1, "any\nany"),
];

pub fn register(eng: &Engine) {
    for (op, nr, nw, sig) in REGS {
        unsafe { kvlang_rwirextRegister(eng.kv, cs(op).as_ptr(), *nr, *nw, cs(sig).as_ptr()) };
    }
}

/// 主导驱动循环遇到 rwir 就分派。
pub fn dispatch(eng: &Engine, op: &str, pc: &str) {
    match op {
        "print" | "println" | "cerr" => io::print_line(eng, pc),
        "input" => io::input(eng, pc),
        "json.to" => json::to(eng, pc),
        "json.from" => json::from(eng, pc),
        "http.call" => http::call(eng, pc),
        "llm.call" => {
            let userinput = eng.read0(pc);
            let entry = llm::codegen(eng, &userinput);
            eng.set_kv(&eng.write0(pc), &entry);
        }
        "byteseek.run" => byteseek_run(eng, &eng.read0(pc)),
        "shell.run" => {
            let out = tool_run("shell", "bash", "-c", &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        "python.run" => {
            let out = tool_run("python", "python3", "-c", &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        "kvlanglayout.vet" => {
            let out = kvlayout::vet(eng, &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        "kvlanglayout.layout" => {
            let out = kvlayout::layout(eng, &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        "kvlanglayout.src" => {
            let out = kvlayout::src(eng, &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        other => eprintln!("[byteseek] 未知 rwir: {other} @ {pc}"),
    }
}

/// byteseek.run(entry)：把生成的 kv 程序作为嵌套 vthread 跑到结束（可重入 run_fn）。
fn byteseek_run(eng: &Engine, entry: &str) {
    if entry.is_empty() || entry.starts_with("error") {
        eprintln!("[byteseek] 跳过执行（生成失败）：{entry}");
        return;
    }
    eng.run_fn(entry);
}

/// shell.run / python.run 共用：跑一段代码，stdout(+stderr) 截断后作为字符串返回。
fn tool_run(label: &str, prog: &str, flag: &str, code: &str) -> String {
    println!("\n🔧 {label}:\n{code}");
    let out = Command::new(prog).arg(flag).arg(code).output();
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
    result
}
