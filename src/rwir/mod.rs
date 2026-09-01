//! byteseek 运行时胶水：复用 kvlang_rs 的 Engine / ffi / 纯净 rwir（term/json/http/kvlanglayout），
//! 只叠加 agent 专属 rwir 与模式2 驱动循环：
//!   byteseek·run : 把生成的 kv 程序作为嵌套 vthread 跑到结束（可重入 run_fn）
//!   shell·run    : 跑一段 bash，stdout(+stderr) 截断后写回槽
//!   python·run   : 跑一段 python3，同上
//!   input        : 委托纯净 input 读行，再把该行落入 byteseek session 的 talk 队列
//!   env·get      : 读环境变量（缺省空串），供 kv 侧自播配置
//! llm·call 与 llm 配置种子（seedllm）皆已是 kv 代码（lib/byteseek/llm.kv、main.kv），Rust 无 llm 逻辑。

use std::ffi::c_char;
use std::process::Command;
use std::ptr::null_mut;

use kvlang_rs::engine::Engine;
use kvlang_rs::ffi::*;
use kvlang_rs::rwir as std_rwir;

const TOOL_CAP: usize = 4000; // shell/python 工具输出截断

/// byteseek 自有 rwir 注册表：(opcode, 读参数, 写参数, 签名)。
pub const REGS: &[(&str, i32, i32, &str)] = &[
    ("byteseek·run", 1, 0, "any"),
    ("shell·run", 1, 1, "any\nany"),
    ("python·run", 1, 1, "any\nany"),
];

/// 先注册 kvlang_rs 纯净集（term/json/http/kvlanglayout），再叠加 byteseek 自有 rwir。
pub fn register(eng: &Engine) {
    std_rwir::register(eng);
    for (op, nr, nw, sig) in REGS {
        unsafe { kvlang_rwirextRegister(eng.kv, cs(op).as_ptr(), *nr, *nw, cs(sig).as_ptr()) };
    }
}

/// 把一段 .kv 源码 layout 进 kvspace（byteseek 自举代码 lib/byteseek/*.kv）。
pub fn layout_src(eng: &Engine, name: &str, src: &str) {
    let mut err = [0u8; 4096];
    let rc = unsafe {
        kvlangLayoutCode(
            cs(src).as_ptr(),
            cs(&eng.dsn).as_ptr(),
            null_mut(),
            0,
            err.as_mut_ptr() as *mut c_char,
            err.len() as u32,
        )
    };
    if rc != 0 {
        eprintln!("[byteseek] layout {name} 失败: {}", cbuf(&err).trim());
        std::process::exit(1);
    }
    println!("[byteseek] layout {name}");
}

/// bootstrap 一个函数并主导驱动其 vthread 直到结束（模式2，全部就地 dispatch）。
/// funcname 按最后一个点切分为 pkg/name（如 "byteseek.main"、"byteseek/session/x.init"）。
/// 可重入：byteseek·run 在 dispatch 里嵌套调用本函数。
pub fn run_fn(eng: &Engine, funcname: &str) {
    let vid = take(unsafe { kvlangRuntimeBootstrap(eng.rt, cs(funcname).as_ptr(), null_mut(), 0) });
    if vid.is_empty() {
        eprintln!("[byteseek] bootstrap {funcname} 失败");
        return;
    }
    let vpc = format!("/vthread/{vid}/\u{2025}pc");
    loop {
        let mut pc: *mut c_char = null_mut();
        let rc = unsafe { kvlangRuntimeExecuteVthread(eng.rt, cs(&vid).as_ptr(), &mut pc) };
        if rc == 0 {
            break; // done
        }
        if rc != 1 {
            let st = eng.get_kv(&format!("/vthread/{vid}/\u{2025}status"));
            let msg = eng.get_kv(&format!("/vthread/{vid}/\u{2025}error/msg"));
            eprintln!("[byteseek] vthread {vid} 错误 rc={rc} {st}: {msg}");
            break;
        }
        let c = take(pc);
        let params = take(unsafe { kvlang_rwirextParams(eng.kv, cs(&c).as_ptr()) });
        let op = params.lines().next().unwrap_or("").to_string();
        dispatch(eng, &op, &c);
        let nxt = take(unsafe { kvlang_rwirextNextPc(cs(&c).as_ptr()) });
        eng.set_kv(&vpc, &nxt);
    }
}

/// 驱动循环遇到 rwir 分派：byteseek 自有就地处理，其余委托 kvlang_rs 纯净 rwir。
pub fn dispatch(eng: &Engine, op: &str, pc: &str) {
    match op {
        "byteseek·run" => byteseek_run(eng, &eng.read0(pc)),
        "shell·run" => {
            let out = tool_run("shell", "bash", "-c", &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        "python·run" => {
            let out = tool_run("python", "python3", "-c", &eng.read0(pc));
            eng.set_kv(&eng.write0(pc), &out);
        }
        "input" => {
            std_rwir::dispatch(eng, "input", pc);
            let line = eng.get_kv(&eng.write0(pc));
            talk_push(eng, "user", &line);
        }
        other => std_rwir::dispatch(eng, other, pc),
    }
}

/// byteseek·run(entry)：把生成的 kv 程序作为嵌套 vthread 跑到结束。
fn byteseek_run(eng: &Engine, entry: &str) {
    if entry.is_empty() || entry.starts_with("error") {
        eprintln!("[byteseek] 跳过执行（生成失败）：{entry}");
        return;
    }
    run_fn(eng, entry);
}

/// talk 队列：byteseek session 下的对话记录（input 落入此处，可寻址/持久）。
fn talk_push(eng: &Engine, role: &str, content: &str) {
    let n: u32 = eng
        .get_kv("/byteseek/session/talk/count")
        .trim()
        .parse()
        .unwrap_or(0);
    eng.set_kv(&format!("/byteseek/session/talk/{n}/role"), role);
    eng.set_kv(&format!("/byteseek/session/talk/{n}/content"), content);
    eng.set_kv("/byteseek/session/talk/count", &(n + 1).to_string());
}

/// shell·run / python·run 共用：跑一段代码，stdout(+stderr) 截断后作为字符串返回。
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
