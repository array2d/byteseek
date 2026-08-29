//! llm 配置种子：启动时把 LLM api 与 key 从环境读入写 kvspace。
//! 代码脑逻辑（llm·call）已下沉 kvlang（lib/byteseek/llm.kv），本模块只留 seed()。

use crate::engine::Engine;

const DEFAULT_URL: &str = "https://api.deepseek.com/chat/completions";

/// 启动时把 LLM api 与 key 种进 kvspace（api 有默认值；key 来自 DEEPSEEK_API_KEY，可为空）。
pub fn seed(eng: &Engine) {
    let api = std::env::var("DEEPSEEK_API_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let key = std::env::var("DEEPSEEK_API_KEY").unwrap_or_default();
    eng.set_kv("/byteseek/llm.api", &api);
    eng.set_kv("/byteseek/llm.key", &key);
}
