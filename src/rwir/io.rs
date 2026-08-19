//! rwir `print` / `println` / `cerr` / `input`。它们不是 kvlang runtime 的 builtin，
//! 由 byteseek 接管（term 化）：print* 经 `kvlang_rwirextPrintLine` 输出；
//! `input` 由 byteseek 自己读 stdin —— kvlang 无 input API，opcode 留给宿主实现。

use std::ffi::c_int;
use std::io::Write;

use crate::engine::Engine;
use crate::ffi::*;

pub fn print_line(eng: &Engine, pc: &str) {
    let mut rawnl: c_int = 0;
    let mut cerr: c_int = 0;
    let line =
        take(unsafe { kvlang_rwirextPrintLine(eng.kv, cs(pc).as_ptr(), &mut rawnl, &mut cerr) });
    let nl = if rawnl == 0 { "\n" } else { "" };
    if cerr != 0 {
        eprint!("{line}{nl}");
        std::io::stderr().flush().ok();
    } else {
        print!("{line}{nl}");
        std::io::stdout().flush().ok();
    }
}

/// input(prompt) -> line：打印 prompt（不换行）→ 读一行 stdin → 落入 talk 队列 → 回填写槽。
/// EOF(Ctrl-D) 视作 "exit"，让主循环优雅退出。
pub fn input(eng: &Engine, pc: &str) {
    let prompt = eng.read0(pc);
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line).unwrap_or(0);
    let line = if n == 0 {
        "exit".to_string()
    } else {
        line.trim_end_matches(['\n', '\r']).to_string()
    };
    eng.talk_push("user", &line);
    eng.set_kv(&eng.write0(pc), &line);
}
