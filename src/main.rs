//! byteseek —— KV 原生 agent substrate。
//!
//! agent 的"自己"（代码/状态/记忆/执行）整个放进一棵可寻址、可持久、可自改的 kvspace(redis) 树。
//! 本进程启动即：连 kvspace → layout 全部内嵌 lib/byteseek/*.kv → 跑 byteseek.init 种语法速览
//! → bootstrap byteseek.main 进入 REPL（input 等 stdin、llm.call 生成 kv、byteseek.run 执行）。

mod engine;
mod ffi;
mod rwir;

use std::ffi::c_char;

use engine::Engine;
use ffi::*;

include!(concat!(env!("OUT_DIR"), "/embedded_kv.rs"));

/// 把一段 .kv 源码 layout 进 kvspace（内存源码，内嵌的 lib/byteseek/*.kv）。
fn layout_src(dsn: &str, src: &str) {
    let (mut entry, mut err) = ([0u8; 512], [0u8; 4096]);
    let rc = unsafe {
        kvlangLayoutCode(
            cs(src).as_ptr(),
            cs(dsn).as_ptr(),
            entry.as_mut_ptr() as *mut c_char,
            entry.len() as u32,
            err.as_mut_ptr() as *mut c_char,
            err.len() as u32,
        )
    };
    if rc != 0 {
        let e = cbuf(&err);
        eprintln!("layout 失败: {}", e.trim());
        std::process::exit(1);
    }
}

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

    // 2) 连接 runtime，注册 rwir
    let rt = unsafe { kvlangRuntimeConnect(cs(&dsn).as_ptr()) };
    if rt.is_null() {
        eprintln!("kvlangRuntimeConnect 失败: {dsn}");
        std::process::exit(1);
    }
    let eng = Engine {
        rt,
        kv,
        dsn: dsn.clone(),
        subs: std::cell::Cell::new(0),
    };
    eng.register();
    rwir::llm::seed(&eng);

    // 3) layout 全部内嵌 lib/byteseek/*.kv（注册 rwfunc/init 到 /lib）
    for (name, src) in EMBEDDED_KV {
        layout_src(&dsn, src);
        println!("[byteseek] layout {name}");
    }

    // 4) 跑 byteseek.init，把语法速览种进 /lib/byteseek.kvlangbrief
    eng.run_fn("byteseek.init");
    let brief = eng.get_kv("/lib/byteseek.kvlangbrief");
    println!(
        "[byteseek] kvlangbrief 已种入 /lib/byteseek.kvlangbrief（{} 字符）",
        brief.chars().count()
    );
    if brief.trim().is_empty() {
        eprintln!("[byteseek] 警告：/lib/byteseek.kvlangbrief 为空");
    }

    // 5) bootstrap byteseek.main 进入 REPL
    eng.run_fn("byteseek.main");

    unsafe {
        kvlangRuntimeDisconnect(rt);
        kvspaceClose(kv);
    };
}
