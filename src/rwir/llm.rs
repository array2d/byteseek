//! llm 配置种子：启动时把 LLM api 与 key 从环境读入写 kvspace。
//! 代码脑逻辑（llm·call）已下沉 kvlang（lib/byteseek/llm.kv），本模块只留 seed()。

use crate::engine::Engine;

const DEFAULT_URL: &str = "https://api.deepseek.com/chat/completions";

/// 启动时把 LLM api/key/print 种进 kvspace（api 有默认值；key 来自 DEEPSEEK_API_KEY，可为空；
/// print 默认 "1" 开，BYTESEEK_LLM_PRINT=0 关）。
pub fn seed(eng: &Engine) {
    let api = std::env::var("DEEPSEEK_API_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let key = std::env::var("DEEPSEEK_API_KEY").unwrap_or_default();
    let print = std::env::var("BYTESEEK_LLM_PRINT").unwrap_or_else(|_| "1".to_string());
    eng.set_kv("/byteseek/llm.api", &api);
    eng.set_kv("/byteseek/llm.key", &key);
    eng.set_kv("/byteseek/llm.print", &print);
}
