//! byteseek —— KV 原生 agent substrate。
//!
//! 不是"用 kvlang 写 agent 框架"，而是把 agent 的"自己"整个放进一棵可寻址、可持久、
//! 可自改的 kvspace(redis) 树：对话历史、当前动作、执行到哪一步(vthread pc) 全在树中。
//! llm / shell / python / agent 是注册进 kvlang runtime 的 rwir（一等公民，见 `rwir/`）；
//! agentloop.kv 是"相对固定的执行逻辑"，既是代码也是数据，同住这棵树。
//!
//! 本进程 = corebrain：把 .kv 布局进 redis → 注册 rwir → bootstrap → 主导驱动 vthread
//! （模式2，仿 rust/term），遇 rwir 就地处理。

mod engine;
mod ffi;
mod rwir;

use std::cell::Cell;
use std::ffi::c_char;
use std::process::Command;

use engine::{Engine, DEFAULT_MODEL};
use ffi::*;

/// LLM 调用参数也是 KV 数据：种入 `/byteseek/llm/*`（可寻址/持久/运行时自改）。
/// 完整字段与含义见 `rwir::llm::LlmConfig`。
fn seed_llm_config(eng: &Engine, api_key: &str) {
    let d: &[(&str, &str)] = &[
        ("url", "https://api.deepseek.com/chat/completions"),
        ("api_key", api_key),
        ("model", DEFAULT_MODEL),
        ("temperature", "0.2"),
        ("top_p", "1.0"),
        ("max_tokens", "4096"),
        ("frequency_penalty", "0.0"),
        ("presence_penalty", "0.0"),
        ("stop", ""),
        ("seed", ""),
        ("timeout_s", "120"),
    ];
    for (k, v) in d {
        eng.set_kv(&format!("/byteseek/llm/{k}"), v);
    }
}

fn main() {
    let dsn = std::env::var("KVSPACE").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let api_key = std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY 未设置");
    let manifest = env!("CARGO_MANIFEST_DIR");
    // 提示词与执行逻辑都是 KV 文件：prompt.kv 先 layout（注册 seed_prompts），
    // agentloop.kv 的 init 里再 seed_prompts() + agentloop()。
    let promptfile =
        std::env::var("BYTESEEK_PROMPT").unwrap_or_else(|_| format!("{manifest}/agent/prompt.kv"));
    let kvfile = std::env::var("BYTESEEK_AGENT")
        .unwrap_or_else(|_| format!("{manifest}/agent/agentloop.kv"));
    let max_steps: u32 = std::env::var("BYTESEEK_MAX_STEPS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(12);

    let task: String = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let task = if task.trim().is_empty() {
        "用 shell 查出当前系统内核版本和 CPU 型号，再用 python 算 1 到 100 的和，最后汇总成一句话。"
            .to_string()
    } else {
        task
    };

    // 1) 清空 kvspace（用 durable kvspace CLI，不用 redis-cli）
    let _ = Command::new("kvspace")
        .args(["--kvspace", &dsn, "clear"])
        .status();

    // 2) 布局 KV 文件进 redis（注册 rwfunc 到 /lib）。prompt.kv 先于 agentloop.kv。
    let layout = |file: &str| {
        let mut entry = [0u8; 512];
        let mut err = [0u8; 512];
        let rc = unsafe {
            kvlang_layout_file(
                cs(file).as_ptr(),
                cs(&dsn).as_ptr(),
                entry.as_mut_ptr() as *mut c_char,
                512,
                err.as_mut_ptr() as *mut c_char,
                512,
            )
        };
        if rc != 0 {
            let e = String::from_utf8_lossy(&err);
            eprintln!("layout 失败 ({file}): {}", e.trim_end_matches('\0').trim());
            std::process::exit(1);
        }
    };
    layout(&promptfile);
    layout(&kvfile);

    // 3) 连接 runtime，注册 rwir
    let rt = unsafe { kvlang_rt_connect(cs(&dsn).as_ptr()) };
    if rt.is_null() {
        eprintln!("kvlang_rt_connect 失败: {dsn}");
        std::process::exit(1);
    }
    let kv = unsafe { kvlang_rt_kv(rt) };
    let eng = Engine {
        rt,
        conn: RwextConn { kv },
        max_steps,
        subs: Cell::new(0),
    };
    eng.register();
    seed_llm_config(&eng, &api_key);

    // 4) 播种 session，指定 sid 供 agentloop 读取
    let sid = "main";
    eng.seed_session(sid, &task);
    eng.mkindex(&format!("/session/{sid}"));

    println!("════════════════════════════════════════════════════════");
    println!("byteseek · KV 原生 agent  |  kvspace={dsn}");
    println!("任务: {task}");
    println!("════════════════════════════════════════════════════════");

    // 5) bootstrap init（顶层 call 包裹 agentloop）并主导驱动
    eng.run_entry(sid);

    println!("\n════════════════════════════════════════════════════════");
    println!(
        "✅ 最终答案:\n{}",
        eng.get_kv(&format!("/session/{sid}/final"))
    );
    println!("════════════════════════════════════════════════════════");
    println!("（用 `kvspace tree /session/{sid}` 可查看 agent 全部状态）");

    unsafe { kvlang_rt_disconnect(rt) };
}
