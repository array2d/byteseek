// 链接 byteseek/lib 下的 durable(redis) 版三方 .so：
//   kvspace_durable —— byteseek 自持 kvspace 句柄（KV 存取 + TLV）
//   kvlang_runtime  —— 模式2 执行 + rwirext 宿主
//   kvlang_layout   —— .kv 编译入库
// 与 kvlang/bin 下的 shm 版并存互不影响。
fn main() {
    let lib = format!("{}/lib", env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-link-search=native={lib}");
    println!("cargo:rustc-link-lib=dylib=kvspace_durable");
    println!("cargo:rustc-link-lib=dylib=kvlang_runtime");
    println!("cargo:rustc-link-lib=dylib=kvlang_layout");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{lib}");
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    println!("cargo:rerun-if-changed=lib");
}
