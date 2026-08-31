// 链接 /usr/lib 下的三方 .so（byteseek 依赖 kvlang_rs，但其 build.rs 链接指令不传播，
// 故本 crate 须自带全部链接标志，解析 kvlang_rs::ffi 引用的 C 符号）：
//   kvspace_durable —— byteseek 自持 kvspace 句柄（KV 存取 + TLV）
//   kvlang_runtime  —— 模式2 执行 + rwirext 宿主
//   kvlanglayout    —— .kv 编译入库
// 并把 lib/**/*.kv（现仅 lib/byteseek/ 自举代码；http.kv 已下沉 runtime-rs stdlib）
// 全部 include_str! 进二进制（EMBEDDED_KV），启动时 layout 进 kvspace。
use std::path::{Path, PathBuf};

fn collect_kv(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_kv(&p, files);
        } else if p.extension().is_some_and(|x| x == "kv") {
            files.push(p);
        }
    }
}

fn main() {
    let manifest = env!("CARGO_MANIFEST_DIR");

    println!("cargo:rustc-link-search=native=/usr/lib");
    println!("cargo:rustc-link-lib=dylib=kvspace_durable");
    println!("cargo:rustc-link-lib=dylib=kvlang_runtime");
    println!("cargo:rustc-link-lib=dylib=kvlanglayout");

    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags"); // rpath 转 DT_RPATH，传递解析子依赖
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib");

    let libdir = format!("{manifest}/lib");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_kv(Path::new(&libdir), &mut files);
    files.sort();
    let mut code = String::from("pub static EMBEDDED_KV: &[(&str, &str)] = &[\n");
    for p in &files {
        let rel = p.strip_prefix(&libdir).unwrap_or(p);
        let name = rel.to_string_lossy().trim_start_matches('/').to_string();
        let abs = p.to_string_lossy();
        code.push_str(&format!("    ({name:?}, include_str!({abs:?})),\n"));
        println!("cargo:rerun-if-changed={abs}");
    }
    code.push_str("];\n");
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{out}/embedded_kv.rs"), code).unwrap();
    println!("cargo:rerun-if-changed={libdir}");
}
