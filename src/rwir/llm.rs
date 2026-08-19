//! rwir `llm.call(userinput) -> entry`：代码脑。
//! 系统提示 = /byteseek/prompt/system（怎么生成）+ /lib/byteseek.kvlangbrief（kvlang 语法速览）。
//! 调 LLM 产出 <name>/<kv>，包进 lib byteseek { lib session { lib NAME {…} } } 后 layout 入库，
//! 返回构造出的入口名 byteseek/session/NAME.init（不信任 layout 的全局 find_entry）。
//!
//! LLM 调用参数是 KV 数据，活在 /byteseek/llm.api 与 /byteseek/llm.key（seed() 从环境读入写树）。

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::engine::{Engine, DEFAULT_MODEL};
use crate::rwir::kvlayout;

/// LLM 接口地址与鉴权 key 是 KV 数据：启动时 seed() 从环境读入，写入
/// /byteseek/llm.api 与 /byteseek/llm.key；llm.call 默认取这两个路径（其余参数用内置默认）。
const DEFAULT_URL: &str = "https://api.deepseek.com/chat/completions";

/// 启动时把 LLM api 与 key 种进 kvspace（api 有默认值；key 来自 DEEPSEEK_API_KEY，可为空）。
pub fn seed(eng: &Engine) {
    let api = std::env::var("DEEPSEEK_API_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let key = std::env::var("DEEPSEEK_API_KEY").unwrap_or_default();
    eng.set_kv("/byteseek/llm.api", &api);
    eng.set_kv("/byteseek/llm.key", &key);
}

pub struct LlmConfig {
    pub url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f64,
    pub top_p: f64,
    pub max_tokens: u32,
    pub frequency_penalty: f64,
    pub presence_penalty: f64,
    pub stop: Vec<String>,
    pub seed: Option<i64>,
    pub timeout_s: u32,
}

impl LlmConfig {
    pub fn load(eng: &Engine) -> Self {
        let api = eng.get_kv("/byteseek/llm.api");
        let url = if api.trim().is_empty() {
            DEFAULT_URL.to_string()
        } else {
            api
        };
        LlmConfig {
            url,
            api_key: eng.get_kv("/byteseek/llm.key"),
            model: DEFAULT_MODEL.to_string(),
            temperature: 0.2,
            top_p: 1.0,
            max_tokens: 4096,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            stop: Vec::new(),
            seed: None,
            timeout_s: 120,
        }
    }

    fn body(&self, sys: &str, user: &str) -> String {
        let b = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": sys},
                {"role": "user", "content": user},
            ],
            "temperature": self.temperature,
            "top_p": self.top_p,
            "max_tokens": self.max_tokens,
            "frequency_penalty": self.frequency_penalty,
            "presence_penalty": self.presence_penalty,
            "stream": false,
        });
        let mut b = b;
        if !self.stop.is_empty() {
            b["stop"] = json!(self.stop);
        }
        if let Some(s) = self.seed {
            b["seed"] = json!(s);
        }
        b.to_string()
    }

    /// 发请求，返回 `(http 状态码, 原始响应体)`；curl 本身失败（启动/执行）走 Err。
    fn request(&self, sys: &str, user: &str) -> Result<(u16, String), String> {
        let body = self.body(sys, user);
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
                "-w",
                "\n%{http_code}",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => return Err(format!("curl 启动失败: {e}")),
        };
        if let Some(mut sin) = child.stdin.take() {
            let _ = sin.write_all(body.as_bytes());
        }
        let out = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return Err(format!("curl 执行失败: {e}")),
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let (body, code) = match text.rsplit_once('\n') {
            Some((b, c)) => (b.to_string(), c.trim().parse::<u16>().unwrap_or(0)),
            None => (text.into_owned(), 0),
        };
        Ok((code, body))
    }
}

/// 代码脑入口：llm.call(userinput) -> entry。
pub fn codegen(eng: &Engine, userinput: &str) -> String {
    let brief = eng.get_kv("/lib/byteseek.kvlangbrief");
    let sys = format!(
        "{}\n\n=== kvlang 语法速览 ===\n{}",
        eng.get_kv("/byteseek/prompt/system"),
        brief
    );
    let cfg = LlmConfig::load(eng);
    if cfg.api_key.trim().is_empty() {
        return "error: 未设置 DEEPSEEK_API_KEY（环境变量名大小写敏感），无法调用 LLM".into();
    }
    let user = format!(
        "请把下面的需求翻译成一段 kvlang 程序。严格按系统提示的格式输出，只输出 <name>/<kv> 两段，不要解释：\n\n{userinput}"
    );
    let (code, body) = match cfg.request(&sys, &user) {
        Ok(x) => x,
        Err(e) => return format!("error: {e}"),
    };
    if code != 200 {
        return format!("error: LLM HTTP {code}: {}", body.trim());
    }
    let content = match serde_json::from_str::<Value>(&body) {
        Ok(v) => match v["choices"][0]["message"]["content"].as_str() {
            Some(s) => s.to_string(),
            None => return format!("error: LLM 响应无内容: {}", body.trim()),
        },
        Err(_) => return format!("error: LLM 响应非 JSON: {}", body.trim()),
    };
    println!("\n🧠 [codegen]\n{}", content.trim());

    let (raw_name, kv) = parse_program(&content);
    let kv = if kv.trim().is_empty() {
        // 兜底：没写 <kv> 标签时，剥掉可能的 ```kv 围栏，取整段。
        strip_fence(&content)
    } else {
        kv
    };
    if kv.trim().is_empty() {
        return "error: LLM 未产出 kvlang 程序".into();
    }

    let k = eng.subs.get() + 1;
    eng.subs.set(k);
    let name = {
        let base = sanitize(&raw_name);
        if base.is_empty() {
            format!("s{k}")
        } else {
            format!("{base}_{k}")
        }
    };

    let wrapped = wrap_program(&name, &kv);
    let v = kvlayout::vet(eng, &wrapped);
    if v != "ok" {
        return format!("error: vet 失败: {v}");
    }
    let entry = kvlayout::src(eng, &wrapped);
    if entry.starts_with("error") {
        return entry;
    }
    // 自造入口名，不信任 layout 的全局 find_entry（会返回预先存在的 byteseek.init）。
    format!("byteseek/session/{name}.init")
}

/// 解析 <name>…</name> 与 <kv>…</kv>；取最先出现的标签。
fn parse_program(content: &str) -> (String, String) {
    let tag = |t: &str| -> String {
        let open = format!("<{t}>");
        let close = format!("</{t}>");
        match content.find(&open) {
            Some(i) => {
                let start = i + open.len();
                match content[start..].find(&close) {
                    Some(j) => content[start..start + j].trim().to_string(),
                    None => content[start..].trim().to_string(),
                }
            }
            None => String::new(),
        }
    };
    (tag("name"), tag("kv"))
}

/// 剥掉 ```kv / ``` 围栏。
fn strip_fence(content: &str) -> String {
    let mut s = content.trim().to_string();
    if let Some(rest) = s.strip_prefix("```") {
        s = rest.strip_prefix("kv").unwrap_or(rest).to_string();
    }
    if s.ends_with("```") {
        s = s.trim_end_matches("```").trim_end().to_string();
    }
    s
}

/// 名字转成合法 kvlang 标识符：小写，非字母数字换成 `_`，禁止数字开头（前缀 s）。
fn sanitize(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("s{s}")
    } else {
        s
    }
}

/// 包进 lib byteseek { lib session { lib NAME {…} } }，保证末尾有 main() 调用。
fn wrap_program(name: &str, kv: &str) -> String {
    let body = if kv.lines().any(|l| matches!(l.trim(), "main()" | "main();")) {
        kv.trim().to_string()
    } else {
        format!("{}\nmain()", kv.trim())
    };
    format!(
        "lib byteseek {{\nlib session {{\nlib {name} {{\n{body}\n}}\n}}\n}}"
    )
}
