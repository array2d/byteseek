//! rwir `print` / `println` / `cerr`：行输出。它们不是 kvlang runtime 的 builtin，
//! 由引擎经 `rwext_print_line` 处理（println 补换行，cerr 走 stderr）。

use std::ffi::c_int;

use crate::engine::Engine;
use crate::ffi::*;

pub fn print_line(eng: &Engine, pc: &str) {
    let mut rawnl: c_int = 0;
    let mut cerr: c_int = 0;
    let line =
        take(unsafe { rwext_print_line(eng.conn_ptr(), cs(pc).as_ptr(), &mut rawnl, &mut cerr) });
    let nl = if rawnl == 0 { "\n" } else { "" };
    if cerr != 0 {
        eprint!("{line}{nl}");
    } else {
        print!("{line}{nl}");
    }
}
