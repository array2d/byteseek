# 阶段一研发任务清单

> 日期：2026-08-19（更新）
> 对应 roadmap：`README.md` / `README_CN.md` 的「阶段一 —— corebrain 自造代码，人工辅助扩展」。
> 架构基线：`doc/substrate.md`（已同步当前 src）。

> **架构演进**：corebrain 主循环已从「LLM 决定动作类型 → 分派固定工具（shell/python/agent）」
> 演进为「LLM 直接生成 kvlang 程序 → `byteseek.run` 执行」。旧的 `agent/agentloop.kv`、
> `llm.call(sid)->kind`、`agent.spawn`、`/session/{sid}/*` 已废弃；对应 `mainbrain.kv`、
> `llm.call(userinput)->entry`、`/byteseek/session/talk/*`。

## 阶段一的完成定义（exit criteria）

1. **loop 可靠** —— corebrain 主循环（`mainbrain`）在崩溃、超时、工具报错、长上下文下
   都能稳定推进或可恢复。
2. **rwirext 成体系且高信任** —— 扩展库从当前 term/json/http/shell/python 扩到覆盖真实
   任务的一套库，每个 ext 有测试、有边界、可审计。
3. **corebrain 自造 kv 代码** —— LLM 在运行时生成/改写 kv 代码与提示词，vet 后 layout
   进 `/lib` 并执行（已具备 `llm.call` 生成 + `kvlanglayout.*` 入库，待补齐自改闭环）。

---

## A. loop 稳健化（把「能跑」变「可靠」）

- [ ] **A1 崩溃恢复真正打通。** 现状 `engine.rs::run_fn` 每次 `bootstrap` 全新帧，
  `/vthread/{vid}/‥pc` 只写不读，崩溃恢复停在「理论可行」。做：新增 resume 模式——启动
  时若目标 vthread PC 未走完，从既有 PC 继续 execute。验收：跑到中途 `kill -9`，重启续跑。
- [ ] **A2 run 与 flush 解耦。** 现状 `main.rs` 每次清空并重排 kvspace，使持久化前功尽弃。
  做：区分「新任务（种 session）」与「恢复（读既有 session）」，flush 仅在显式 `--fresh`。
- [ ] **A3 工具报错纳入循环反馈。** 现状 `llm.call` 失败回填 `error: …`，`byteseek.run`
  跳过执行，但无「失败重试/换方案」的结构约束。做：连续失败有界重试与升级。
- [ ] **A4 上下文窗口管理。** 现状 talk 队列（`/byteseek/session/talk/*`）每轮全量塞进
  请求，长任务超长。做：阈值处摘要压缩旧轮，压缩产物落在树、可观测。

## B. rwirext 扩展库（人工辅助建高信任度工具）

- [x] **B1 json 内嵌。** `json.to/from` 已由 go/json 重写为 `src/rwir/json.rs`（单进程），
  单测覆盖子树↔JSON 往返。
- [x] **B2 http 内嵌。** 单 rwir `http.call(method,header,url,body)`（rust 原生 ureq）+
  `lib http` rwfunc 封装（get/post/put/del）。参数类型 `[]char/utf32`（字符串字面量默认编码）。
- [ ] **B3 os 内嵌。** `os.proc`（吸收 shell.run/python.run）→ `os.fs` → `os.net`。
- [ ] **B4 记忆/检索 rwir。** `mem.put / mem.get / mem.search`，跨 session 记忆存 kvspace
  固定子树，供后续任务寻址复用。（注意：与 `os.mem` 无关，`os.mem` 已砍）
- [ ] **B5 rwir 注册与签名规范化。** 现状 `rwir/mod.rs::REGS` 手写常量表 + dispatch match。
  做：把「一个 ext = 子模块 + REGS 项 + dispatch 分支」固化为清单化流程或宏。
- [ ] **B6 每个 rwirext 的信任基线。** 每个 ext 具备：单测、输入/输出边界（截断/超时/路径
  白名单）、失败可观测。

## C. corebrain 自造 kv 代码（阶段一定义能力）

- [x] **C1 运行时 layout + vet + bootstrap 的 rwir。** `kvlanglayout.vet/layout/src` 已注册；
  `llm.call` 生成 kv → vet 闸门 → layout 入库 → 返回入口。
- [x] **C2 生成代码的验证闸门。** `llm.call` 前置 `vet`，失败不注册、回填 `error:` 重写。
- [ ] **C3 提示词/循环的运行时自改。** 给出安全的自改协议（改 `/byteseek/prompt/*` 与重
  layout 主循环的边界与回滚），并让 agent 能主动改写。

## D. LLM 接入层稳健

- [ ] **D1 多 provider 请求体。** 现状 `llm.rs::body` 只发 OpenAI Chat 形状。做：按
  `url`/`model` 选择请求体与响应解析形状（Anthropic 等）。
- [ ] **D2 瞬时错误重试。** `llm.rs` 对超时/5xx/空响应做有界指数退避重试，仍失败才降级。
- [ ] **D3 动作解析健壮化。** `llm.rs` 解析 `<name>/<kv>` 处理多块、缺闭合标签、空块等边界。
- [ ] **D4 token/用量记账。** 每轮 usage 写进 `/byteseek/session/usage/*`。

## E. 可观测与调试

- [ ] **E1 dashboard 接入。** 复用 `kvlang-device-screen` 实时看 `/byteseek/session/*`、
  `/vthread/*`、talk 树与当前生成代码。
- [ ] **E2 结构化运行日志入树。** 关键事件写进 `/byteseek/session/log/*`，崩溃后可复盘。

## F. 测试回归与 CI

- [ ] **F1 脚本化任务回归集。** 一组带确定断言的任务（final 内容或 kvspace 子树状态），
  分层：离线（mock LLM/固定 seed）最小集 + 真实 LLM 冒烟集。`make test` 一键跑最小集。
- [ ] **F2 CI。** F1 最小集接入 CI（无需 API key 的部分）。

## G. 安全与信任

- [ ] **G1 shell/python 沙箱与权限。** 现状 `tool_run` 直接 `bash -c`/`python3 -c`，无沙箱。
  做：可配置执行边界（工作目录限制、命令白/黑名单、危险操作确认）。
- [ ] **G2 密钥不落树、不入日志。** `api_key` 仅走 env，树里与日志里不出现明文。

---

## 建议推进顺序

1. **扩能力**：B3 os（proc→fs→net）→ B6 信任基线 → B4 注册规范化。
2. **上自改闭环**：C3（提示词/循环自改）→ A1+A2（恢复/持久）→ A3/A4。
3. **补稳健与观测**：D1–D4、E1/E2 并行。
4. **收口**：G1/G2 安全基线 + F1/F2 回归/CI，对齐三条 exit criteria 后进入阶段二评审。
