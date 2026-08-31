//! byteseek —— KV 原生 agent substrate。
//!
//! agent 的"自己"（代码/状态/记忆/执行）整个放进一棵可寻址、可持久、可自改的 kvspace(redis) 树。
//! 本进程启动即：连 kvspace → 注册 rwir（kvlang_rs 纯净集 + byteseek 自有）→ layout runtime-rs
//! 内嵌 stdlib 与自举 lib/byteseek/*.kv → 跑 byteseek·init 种语法速览 → 驱动 byteseek·main 进入
//! REPL（input 等 stdin、llm·call 生成 kv、byteseek·run 执行）。
//!
//! Engine / ffi / 纯净 rwir(term/json/http/kvlanglayout) 全部复用 kvlang runtime-rs（kvlang_rs crate），
//! byteseek 仅叠加 agent 专属 rwir 与驱动循环（见 crate::rwir）。
#![allow(non_snake_case)]

mod rwir;

use kvlang_rs::engine::Engine;
use kvlang_rs::ffi::*;
use std::ffi::c_char;

include!(concat!(env!("OUT_DIR"), "/embedded_kv.rs")); // byteseek 自举代码 lib/byteseek/*.kv

fn main() {
    let dsn = std::env::var("KVSPACE").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    // 1) 连接 kvspace（byteseek 自持句柄）并清空
    let kv = unsafe { kvspaceConnect(cs(&dsn).as_ptr()) };
    if kv.is_null() {
        eprintln!("kvspaceConnect 失败: {dsn}");
        std::process::exit(1);
    }
    let mut cerr = [0u8; 256];
    unsafe { kvspaceClear(kv, cerr.as_mut_ptr() as *mut c_char, 256) };

    // 2) 连接 runtime
    let rt = unsafe { kvlangRuntimeConnect(cs(&dsn).as_ptr()) };
    if rt.is_null() {
        eprintln!("kvlangRuntimeConnect 失败: {dsn}");
        std::process::exit(1);
    }
    let eng = Engine {
        rt,
        kv,
        dsn: dsn.clone(),
    };

    // 3) 注册 rwir（纯净集 + byteseek 自有）
    rwir::register(&eng);

    // 4) layout+run runtime-rs 内嵌 stdlib（http/kv/string/… 常量落值），再 layout byteseek 自举代码
    eng.layout_stdlib();
    eng.run_stdlib_init();
    for (name, src) in EMBEDDED_KV {
        rwir::layout_src(&eng, name, src);
    }

    // 5) 跑 byteseek·init，把语法速览种进 /lib/byteseek.kvlangbrief
    rwir::run_fn(&eng, "byteseek·init");
    let brief = eng.get_kv("/lib/byteseek.kvlangbrief");
    println!(
        "[byteseek] kvlangbrief 已种入 /lib/byteseek.kvlangbrief（{} 字符）",
        brief.chars().count()
    );
    if brief.trim().is_empty() {
        eprintln!("[byteseek] 警告：/lib/byteseek.kvlangbrief 为空");
    }

    // 6) 驱动 byteseek·main 进入 REPL
    rwir::run_fn(&eng, "byteseek·main");

    unsafe {
        kvlangRuntimeDisconnect(rt);
        kvspaceClose(kv);
    };
}
