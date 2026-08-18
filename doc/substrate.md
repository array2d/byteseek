# byteseek —— KV 原生 agent substrate

> 日期：2026-08-18
> 实现：`src/`（corebrain 引擎，Rust）+ `agent/agentloop.kv`（执行逻辑）

## 源码结构

```
src/main.rs        入口：清空/布局 kvspace → 连接 runtime → 注册 rwir → 种配置 → 主导驱动
src/ffi.rs         C ABI：layout(cdylib) + runtime(模式2) + rwext
src/engine.rs      Engine：kvspace 读写、消息树、run_entry 主导驱动循环
src/rwir/          四个一等公民 rwir，一模块一职责：
  ├─ llm.rs        llm.call(sid)->kind + LlmConfig（调用参数集，见下）
  ├─ shell.rs      shell.run(sid)
  ├─ python.rs     python.run(sid)
  ├─ agent.rs      agent.spawn(sid)
  ├─ io.rs         print / println / cerr
  └─ mod.rs        注册表 REGS + dispatch 派发 + shell/python 共用 tool_run
```

## 定位

byteseek 不是又一个 agent 框架。区别在于：agent 的"自己"——代码、状态、
记忆、执行进度——全部活在**同一棵可寻址、可持久、可自改的 KV 树**（kvspace，
后端为 redis）里。LLM、shell、python、子 agent 通过注册 rwir 成为这棵树里的
一等公民。

一个进程 = 一个 **corebrain**：把 `.kv` 布局进 redis → 注册 rwir → bootstrap
一条 vthread → 主导驱动执行（kvlang 模式 2），遇到 rwir 就地处理。

## 状态树布局

```
/byteseek/cursid            当前 session id（引擎每次 bootstrap 前写入）
/byteseek/prompt/system     系统提示（由 prompt.kv 的 seed_prompts() 播种，见下）
/byteseek/llm/*             LLM 调用参数（url/model/temperature/…，见下）
/session/{sid}/task         用户任务
/session/{sid}/model        LLM 模型
/session/{sid}/steps        该 session 已用 LLM 轮数（每 session 独立计额）
/session/{sid}/msg/count    对话轮数
/session/{sid}/msg/{i}/role|content   对话历史
/session/{sid}/action/kind|arg        当前动作（llm.call 解析 LLM 输出写入）
/session/{sid}/final        最终答案
/vthread/{vid}/‥pc          执行到哪一步（KV 路径字符串，崩溃可恢复）
```

一个 session 的全部状态可直接观测：`kvspace tree /session/{sid}/`。

## 提示词也是 KV 数据

系统提示不硬编码在 Rust 里，而是作为一个 KV 文件 `agent/prompt.kv`：其中的
`rwfunc seed_prompts()` 把提示词字符串写进 `/byteseek/prompt/system`。agentloop.kv
的顶层是 `seed_prompts()` 后跟 `agentloop()`，两者一起被 layout 包进 `init` 帧——
每次 bootstrap，先播种提示词进树，再进主循环。引擎每轮 `llm.call` 从
`/byteseek/prompt/system` 读系统提示。

于是提示词和执行逻辑一样可寻址、可持久、可运行时自改：改 agent 的"人格/协议"
只需改 prompt.kv，或直接改树里的 `/byteseek/prompt/*`。这也是 corebrain 能
"生成/改写 kv 提示词与代码再执行"的基础。

## LLM 调用参数也是 KV 数据

`llm.call` 的参数不硬编码，同样活在树里 `/byteseek/llm/*`（引擎启动时种入默认，
之后可寻址/持久/运行时自改；`model` 可被 `/session/{sid}/model` 覆盖）。字段取
OpenAI Chat Completions（DeepSeek、各 OpenAI 兼容网关同款）与 Anthropic Messages
的公共参数集，定义在 `rwir::llm::LlmConfig`：

| 键 | 含义 |
|----|------|
| `url` | 接口地址（OpenAI 兼容 `{base}/chat/completions`） |
| `api_key` | 鉴权 `Authorization: Bearer <key>` |
| `model` | 模型名 |
| `temperature` | 采样温度 0~2 |
| `top_p` | 核采样阈值 0~1 |
| `max_tokens` | 生成上限（Anthropic 必填） |
| `frequency_penalty` | 频率惩罚 -2~2 |
| `presence_penalty` | 存在惩罚 -2~2 |
| `stop` | 停止序列（逗号分隔） |
| `seed` | 随机种子（可复现，空则不带） |
| `timeout_s` | 单次请求超时秒 |

换模型/网关/温度只需 `kvspace set /byteseek/llm/model …`，无需改代码重编译。

## 四个 rwir（一等公民）

在 `rwir::register`（`src/rwir/mod.rs` 的 `REGS` 表）里注册到 `/lib/<opcode>`：

| opcode | 读/写 | 引擎处理 |
|--------|-------|----------|
| `llm.call(sid) -> kind` | 1/1 | 读 session 的 system+msg，调 DeepSeek，解析动作块，写 `action/*`，返回动作类型 |
| `shell.run(sid)` | 1/0 | 跑 `action/arg` 里的 bash，输出截断后 append 进 msg |
| `python.run(sid)` | 1/0 | 同上，跑 `python3 -c` |
| `agent.spawn(sid)` | 1/0 | 新建子 session + 新 vthread，跑同一段 agentloop，把子 agent 的 final 摘要回填进父 msg |

`print`/`println`/`cerr` 也注册为 rwir，由引擎经 `rwext_print_line` 处理
（它们不是 kvlang runtime 的 builtin）。

## agentloop.kv —— "相对固定的执行逻辑"

它既是代码也是数据，同住这棵树。核心是一个 while 循环：`llm.call` 决定动作类型，
`string.cmp` 分支到 shell / python / agent / final：

```
rwfunc agentloop() -> () {
	sid <- /byteseek/cursid
	1 -> running
	while (running == 1) {
		llm.call(sid) -> kind
		string.cmp(kind, "shell")  -> csh   if (csh == 0) { shell.run(sid) }
		string.cmp(kind, "python") -> cpy   if (cpy == 0) { python.run(sid) }
		string.cmp(kind, "agent")  -> cag   if (cag == 0) { agent.spawn(sid) }
		string.cmp(kind, "final")  -> cfi   if (cfi == 0) { 0 -> running }
	}
}
seed_prompts()
agentloop()
```

corebrain 生成/持有这段 kv 代码并立即执行；kvspace 承担 LLM 上下文的管理
（对话历史即 KV 子树）。

## 两个实现要点（踩坑记录）

**1. 带 scope 的函数必须在被调用的嵌套帧里执行。**
layout 禁止顶层 `while`，会把顶层语句包进 `init` 帧。若直接 bootstrap 带 scope
的函数作为顶层 vthread 帧，其 while 的 scope PC（如 `/vthread/1/_while_2/...`）
无法经该帧的 extindex 解析，会静默失败。正确做法：bootstrap `init`（无参），
让 `init` 里的 `agentloop()` 在嵌套调用帧执行——此时 scope PC 形如
`/vthread/1/[1,0]/_while_2/...`，正确寻址。见 `run_entry`。

**2. sid 经固定 KV 路径传入，不经 call 实参。**
runtime 的 `resolve_read_path` 把 `/`-开头的实参判为字面量并返回 NULL，
故 `agentloop(/byteseek/cursid)` 无法把路径值绑进帧槽（槽为 None）。改为
agentloop 无参、函数体首行 `sid <- /byteseek/cursid` 直接读绝对路径。
引擎在每次 `run_entry` 前把当前 session id 写入 `/byteseek/cursid`。

## 运行

```bash
cargo build --release
./target/release/byteseek "用 shell 执行 echo hello，把输出作为最终答案。"
kvspace --kvspace redis://127.0.0.1:6379 tree /session/main/   # 查看 agent 全部状态
```

环境变量：`KVSPACE`（默认 `redis://127.0.0.1:6379`）、`DEEPSEEK_API_KEY`、
`BYTESEEK_AGENT`（默认 `agent/agentloop.kv`）、`BYTESEEK_PROMPT`（默认
`agent/prompt.kv`）、`BYTESEEK_MAX_STEPS`（每 session 最大 LLM 轮数，默认 12）。

## 已验证

- 直接 final（1 轮 LLM）
- shell.run（echo → final）
- python.run + agent.spawn（父派生子 agent，子用 python 算 `2**10`，回填 `1024`）
- 多步工具链（shell 查 CPU 核数+内存 → python 算人均内存 → 一句话汇总 final）
- 提示词经 prompt.kv 播种进 `/byteseek/prompt/system`，引擎从树读取
- LLM 参数种入 `/byteseek/llm/*`，`llm.call` 每轮从树装配请求
- 重构为 `src/rwir/` 分组后，shell→final 全链路复测通过

子 agent 是独立 vthread + 独立 session、独立轮数额度，执行进度各自可观测，
跑同一段 agentloop。
