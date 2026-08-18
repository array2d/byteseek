//! rwir `llm.call(sid) -> kind`：按 session 的对话历史 + 系统提示调 LLM，
//! 解析动作块写入 `action/*`，返回动作类型。
//!
//! LLM 的调用参数不硬编码：和提示词一样是 **KV 数据**，活在 `/byteseek/llm/*`
//! （可寻址 / 可持久 / 可运行时自改）。字段取 OpenAI Chat Completions（DeepSeek、
//! 各 OpenAI 兼容网关同款）与 Anthropic Messages 的公共参数集，逐项列举如下。

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::engine::{Engine, DEFAULT_MODEL};

/// LLM 调用配置。存于 `/byteseek/llm/<field>`；`model` 可被 `/session/{sid}/model` 覆盖。
pub struct LlmConfig {
    pub url: String,            // 接口地址（OpenAI 兼容：{base}/chat/completions）
    pub api_key: String,        // 鉴权：Authorization: Bearer <key>
    pub model: String,          // 模型名
    pub temperature: f64,       // 采样温度 0~2（越高越发散）
    pub top_p: f64,             // 核采样阈值 0~1（与 temperature 二选一为宜）
    pub max_tokens: u32,        // 生成 token 上限（Anthropic 为必填）
    pub frequency_penalty: f64, // 频率惩罚 -2~2（抑制重复词）
    pub presence_penalty: f64,  // 存在惩罚 -2~2（鼓励换话题）
    pub stop: Vec<String>,      // 停止序列（命中即截断），逗号分隔
    pub seed: Option<i64>,      // 随机种子（可复现），空则不带
    pub timeout_s: u32,         // 单次请求超时（秒）
}

impl LlmConfig {
    /// 从树读取；缺省时回落到内置默认（DeepSeek）。
    pub fn load(eng: &Engine, sid: &str) -> Self {
        let g = |k: &str| eng.get_kv(&format!("/byteseek/llm/{k}"));
        let s_or = |v: String, d: &str| {
            if v.trim().is_empty() {
                d.to_string()
            } else {
                v
            }
        };
        let f_or = |k: &str, d: f64| g(k).trim().parse().unwrap_or(d);

        let model_sess = eng.get_kv(&format!("/session/{sid}/model"));
        let model = if model_sess.trim().is_empty() {
            s_or(g("model"), DEFAULT_MODEL)
        } else {
            model_sess
        };
        let api_key = {
            let k = g("api_key");
            if k.trim().is_empty() {
                std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()
            } else {
                k
            }
        };
        LlmConfig {
            url: s_or(g("url"), "https://api.deepseek.com/chat/completions"),
            api_key,
            model,
            temperature: f_or("temperature", 0.2),
            top_p: f_or("top_p", 1.0),
            max_tokens: g("max_tokens").trim().parse().unwrap_or(4096),
            frequency_penalty: f_or("frequency_penalty", 0.0),
            presence_penalty: f_or("presence_penalty", 0.0),
            stop: g("stop")
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
            seed: g("seed").trim().parse().ok(),
            timeout_s: g("timeout_s").trim().parse().unwrap_or(120),
        }
    }

    fn body(&self, sys: &str, msgs: &[(String, String)]) -> String {
        let mut arr: Vec<Value> = vec![json!({"role":"system","content":sys})];
        for (r, c) in msgs {
            let role = if r == "assistant" {
                "assistant"
            } else {
                "user"
            };
            arr.push(json!({"role":role,"content":c}));
        }
        let mut b = json!({
            "model": self.model,
            "messages": arr,
            "temperature": self.temperature,
            "top_p": self.top_p,
            "max_tokens": self.max_tokens,
            "frequency_penalty": self.frequency_penalty,
            "presence_penalty": self.presence_penalty,
            "stream": false,
        });
        if !self.stop.is_empty() {
            b["stop"] = json!(self.stop);
        }
        if let Some(s) = self.seed {
            b["seed"] = json!(s);
        }
        b.to_string()
    }

    /// 经 curl 子进程发请求（避免引入 async 依赖），返回助手文本。
    fn request(&self, sys: &str, msgs: &[(String, String)]) -> String {
        let body = self.body(sys, msgs);
        let child = Command::new("curl")
            .args([
                "-s",
                "--max-time",
                &self.timeout_s.to_string(),
                &self.url,
                "-H",
                "Content-Type: application/json",
                "-H",
                &format!("Authorization: Bearer {}", self.api_key),
                "--data-binary",
                "@-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => return format!("<final>\ncurl 启动失败: {e}\n</final>"),
        };
        if let Some(mut sin) = child.stdin.take() {
            let _ = sin.write_all(body.as_bytes());
        }
        let out = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return format!("<final>\ncurl 执行失败: {e}\n</final>"),
        };
        let text = String::from_utf8_lossy(&out.stdout);
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => v["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("<final>\nLLM 无内容: {}\n</final>", text.trim())),
            Err(_) => format!("<final>\nLLM 响应解析失败: {}\n</final>", text.trim()),
        }
    }
}

/// rwir 入口：llm.call(sid) -> kind。
pub fn call(eng: &Engine, sid: &str) -> String {
    // 轮数每 session 独立，存在树里（子 agent 各拿满额度，不共享父预算）。
    let n = eng
        .get_kv(&format!("/session/{sid}/steps"))
        .trim()
        .parse()
        .unwrap_or(0)
        + 1;
    eng.set_kv(&format!("/session/{sid}/steps"), &n.to_string());
    if n > eng.max_steps {
        let ans = "已达最大轮数上限，停止。";
        eng.append_msg(sid, "assistant", ans);
        eng.set_action(sid, "final", ans);
        return "final".into();
    }
    let cfg = LlmConfig::load(eng, sid);
    let sys = eng.system_prompt();
    let msgs = eng.read_msgs(sid);
    let content = cfg.request(&sys, &msgs);
    println!("\n🧠 [{sid} · 第{n}轮]\n{}", content.trim());
    eng.append_msg(sid, "assistant", &content);
    let (kind, arg) = parse_action(&content);
    eng.set_action(sid, &kind, &arg);
    kind
}

/// 解析 LLM 输出里的动作块：取最先出现的 <tag>…</tag>。
/// 容错：找到开标签但缺闭标签时（模型偶尔漏写），取到文本末尾。
fn parse_action(content: &str) -> (String, String) {
    let mut best: Option<(usize, &str, String)> = None;
    for tag in ["shell", "python", "agent", "final"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        if let Some(i) = content.find(&open) {
            let start = i + open.len();
            let inner = match content[start..].find(&close) {
                Some(j) => content[start..start + j].trim().to_string(),
                None => content[start..].trim().to_string(),
            };
            if best.as_ref().map_or(true, |(bi, _, _)| i < *bi) {
                best = Some((i, tag, inner));
            }
        }
    }
    match best {
        Some((_, tag, inner)) => (tag.to_string(), inner),
        None => ("final".to_string(), content.trim().to_string()),
    }
}
