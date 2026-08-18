// 链接 byteseek/lib 下的 durable(redis) 版 kvlang runtime + layout。
// 这两个 .so 由 kvlang C runtime / Rust layout crate 以 kvspace_durable 后端构建而来，
// 与 kvlang/bin 下的 shm 版并存互不影响。
fn main() {
    let lib = format!("{}/lib", env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-link-search=native={lib}");
    println!("cargo:rustc-link-lib=dylib=kvlang_runtime");
    println!("cargo:rustc-link-lib=dylib=kvlang_layout");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{lib}");
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    println!("cargo:rerun-if-changed=lib");
}
