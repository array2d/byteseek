// 链接 byteseek/libso 下的 durable(redis) 版三方 .so：
//   kvspace_durable —— byteseek 自持 kvspace 句柄（KV 存取 + TLV）
//   kvlang_runtime  —— 模式2 执行 + rwirext 宿主
//   kvlang_layout   —— .kv 编译入库
// 并把 lib/byteseek/*.kv 全部 include_str! 进二进制（EMBEDDED_KV），启动时 layout 进 kvspace。
use std::path::PathBuf;

fn main() {
    let manifest = env!("CARGO_MANIFEST_DIR");

    let libso = format!("{manifest}/libso/lib");
    println!("cargo:rustc-link-search=native={libso}");
    println!("cargo:rustc-link-lib=dylib=kvspace_durable");
    println!("cargo:rustc-link-lib=dylib=kvlang_runtime");
    println!("cargo:rustc-link-lib=dylib=kvlang_layout");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{libso}");
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    println!("cargo:rerun-if-changed=libso");

    let kvdir = format!("{manifest}/lib/byteseek");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&kvdir)
        .unwrap_or_else(|e| panic!("read_dir {kvdir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "kv"))
        .collect();
    files.sort();
    let mut code = String::from("pub static EMBEDDED_KV: &[(&str, &str)] = &[\n");
    for p in &files {
        let name = p.file_name().unwrap().to_string_lossy();
        let abs = p.to_string_lossy();
        code.push_str(&format!("    ({name:?}, include_str!({abs:?})),\n"));
        println!("cargo:rerun-if-changed={abs}");
    }
    code.push_str("];\n");
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{out}/embedded_kv.rs"), code).unwrap();
    println!("cargo:rerun-if-changed={kvdir}");
}
