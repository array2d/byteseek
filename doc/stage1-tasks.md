# 阶段一研发任务清单

> 日期：2026-08-19
> 对应 roadmap：`README.md` / `README_CN.md` 的「阶段一 —— corebrain 自造代码，人工辅助扩展」。
> 架构基线：`doc/substrate.md`。

## 阶段一的完成定义（exit criteria）

阶段一以三条能力同时成立为「完成」：

1. **loop 可靠** —— corebrain 的主循环（`agent/agentloop.kv`）在崩溃、超时、工具报错、
   长上下文下都能稳定推进或可恢复，而非仅在顺利路径上「能跑」。
2. **rwirext 成体系且高信任** —— 扩展能力从当前 4 个（`llm`/`shell`/`python`/`agent`）
   扩到覆盖真实任务的一套库，每个 ext 有测试、有边界、可审计。
3. **corebrain 自造 kv 代码** —— corebrain 能在运行时生成 / 改写 kv 代码与提示词，
   layout 进 `/lib` 并执行，而不只是跑人写死的 `agentloop.kv`。

当前 `doc/substrate.md`「已验证」清单只覆盖了 1 的顺利路径与 2 的最小集，3 尚未落地。

---

## A. loop 稳健化（把「能跑」变「可靠」）

- [ ] **A1 崩溃恢复真正打通。** 现状 `engine.rs::run_entry` 每次都 `bootstrap` 全新
  `init` 帧，`/vthread/{vid}/‥pc` 只写不读，崩溃恢复停在「理论可行」。做：新增 resume
  模式——启动时若目标 session 的 vthread PC 未走到结束，则从既有 PC 继续 execute，
  而非重种。验收：跑到中途 `kill -9`，重启后从断点续跑并给出正确 final。
- [ ] **A2 run 与 flush 解耦。** 现状 `main.rs` 每次清空并重排 kvspace，使 A1 的持久化前功尽弃。
  做：区分「新任务（种 session）」与「恢复（读既有 session）」两条入口，flush 仅在显式
  `--fresh` 时发生。验收：不带 `--fresh` 重跑同一 sid 时状态延续。
- [ ] **A3 工具报错纳入循环反馈。** 现状 `rwir/mod.rs::tool_run` 把 stderr / 非零 exit
  拼进输出回填 msg，但循环无「失败重试 / 换方案」的结构约束，全靠提示词自觉。做：在
  action 里记录 `exit_code`，提示词与循环对连续失败做有界重试与升级（换工具 / final 报错）。
  验收：构造必失败命令，agent 能读报错、改方案、在有界轮内收敛。
- [ ] **A4 上下文窗口管理。** 现状 `engine.rs::read_msgs` 每轮把全部 msg 子树塞进请求，
  长任务必然超长。做：msg 树增长到阈值时做摘要压缩（旧轮归并为 summary 节点，保留近 N 轮
  原文），压缩产物同样落在 `/session/{sid}/msg/*`、可观测。验收：长多步任务不因上下文超限失败。
- [ ] **A5 轮数预算与子 agent 预算传递。** 现状 `llm.rs::call` 每 session 独立 `max_steps`，
  子 agent 各拿满额（`agent.rs::spawn`），深层派生可能预算爆炸。做：父向子传递剩余预算上限，
  树上记账。验收：多层派生的总 LLM 轮数有全局上界且可观测。

## B. rwirext 扩展库（人工辅助建高信任度工具）

- [ ] **B1 文件读写 / 编辑 rwir。** 现状只有裸 `shell`，无结构化文件操作。做：`file.read` /
  `file.write` / `file.edit`（按锚点替换，非整文件覆盖），路径与 diff 记录进树。验收：
  agent 能对一个源文件做精确定点修改并复核 diff。
- [ ] **B2 HTTP / 抓取 rwir。** 做：`http.get` / `http.post`（复用 `llm.rs` 的 curl 子进程模式），
  响应截断策略与 `TOOL_CAP` 一致。验收：agent 能拉取一个 URL 并基于内容推进。
- [ ] **B3 记忆 / 检索 rwir。** 做：`mem.put` / `mem.get` / `mem.search`，把跨 session 记忆
  存进 kvspace 固定子树（如 `/byteseek/mem/*`），供后续任务寻址复用。验收：一个 session 写入的
  事实能被另一 session 检索到。
- [ ] **B4 rwir 注册与签名规范化。** 现状 `rwir/mod.rs::REGS` 是手写常量表，新增 ext 要改多处
  （REGS + dispatch + 子模块）。做：把「一个 ext = 一个子模块 + 一条注册项 + 一个 dispatch 分支」
  固化为清单化流程或宏，降低新增成本与出错面。验收：新增一个 ext 的改动集中、可模板化。
- [ ] **B5 每个 rwirext 的信任基线。** 「高信任度」= 每个 ext 具备：单测、输入/输出边界（截断、
  超时、路径白名单）、失败可观测。做：为 B1–B3 与既有 4 个 ext 补齐上述基线。验收：见 F1 的回归集覆盖。

## C. corebrain 自造 kv 代码（阶段一定义能力）

- [ ] **C1 运行时 layout + bootstrap 的 rwir。** 这是「自造代码」的地基。现状 layout 只在
  `main.rs` 启动时对固定 `.kv` 做一次。做：`kv.layout(src) -> fnname` 把 corebrain 生成的
  kv 源布局进 `/lib`，`kv.call(fnname)` 派生 vthread 执行。验收：agent 在一次会话内生成一小段
  rwfunc、注册、并调用它得到正确结果。
- [ ] **C2 提示词 / 循环的运行时自改。** 现状 `agent/prompt.kv` 的 `seed_prompts()` 只在
  init 播种一次；`substrate.md` 已声明提示词与 `agentloop` 皆可寻址自改，但无 agent 主动改写的
  路径。做：给出安全的自改协议（改 `/byteseek/prompt/*` 与重 layout `agentloop` 的边界与回滚）。
  验收：agent 改写自身系统提示后，下一轮从树读到新提示并按其行为。
- [ ] **C3 自造代码的验证闸门。** 自造 kv 代码必须先过 `vet` / 干跑再纳入执行，避免把坏代码
  写进正在跑的循环。做：C1 的 layout 前置 `kvlang vet` 校验，失败不注册、回填报错给 corebrain 重写。
  验收：故意生成语法错误的 kv 代码时被拦截并触发重写，而非污染 `/lib`。

## D. LLM 接入层稳健

- [ ] **D1 多 provider 请求体。** 现状 `llm.rs::body` 只发 OpenAI Chat 形状；`LlmConfig` 注释
  声称兼顾 Anthropic Messages，但 Anthropic 的 `system` 独立字段、`max_tokens` 必填、响应结构
  均不同。做：按 `url`/`model` 选择请求体与响应解析形状。验收：切到 Anthropic 端点可正常跑通。
- [ ] **D2 瞬时错误重试。** 现状 `llm.rs::request` 对 curl 失败 / 解析失败直接返回一个伪
  `<final>` 错误块，等于把网络抖动当成任务结束。做：对超时 / 5xx / 空响应做有界指数退避重试，
  仍失败才降级。验收：注入一次瞬时失败，循环能重试并继续。
- [ ] **D3 动作解析健壮化。** 现状 `llm.rs::parse_action` 取最先出现的块、缺闭合标签取到末尾。
  做：处理多块（约束只取一个并告警）、代码块内含相似标签、空块等边界。验收：针对构造的畸形输出
  有确定行为，不误判动作类型。
- [ ] **D4 token / 用量记账。** 做：把每轮 usage 写进 `/session/{sid}/usage/*`，供预算（A5）与
  可观测使用。验收：`kvspace tree` 能看到累计用量。

## E. 可观测与调试

- [ ] **E1 dashboard 接入。** 复用 `kvlang-device-screen`，实时看 `/session/*`、`/vthread/*`、
  msg 树与当前 action。验收：一次任务全过程可在 dashboard 回放。
- [ ] **E2 结构化运行日志入树。** 现状用 `println!` 打到 stdout，重启即丢。做：关键事件
  （每轮 kind、工具 exit、重试、压缩）写进 `/session/{sid}/log/*`。验收：崩溃后仍可从树复盘。

## F. 测试回归与 CI

- [ ] **F1 脚本化任务回归集。** 仿 kvlang 的 tutorial 回归：一组带确定断言的任务（final 内容或
  kvspace 子树状态），覆盖 A/B/C/D 的关键路径。做：可离线（mock LLM / 固定 seed）跑的最小集 +
  需真实 LLM 的冒烟集分层。验收：`make test` 一键跑最小集全绿。
- [ ] **F2 CI。** 把 F1 最小集接入 CI（无需 API key 的部分）。验收：PR 触发自动回归。

## G. 安全与信任

- [ ] **G1 shell / python 沙箱与权限。** 现状 `tool_run` 直接 `bash -c` / `python3 -c`，无沙箱、
  无确认。做：可配置的执行边界（工作目录限制、命令白/黑名单、危险操作确认或干跑）。验收：越界命令
  被拦截或需确认。
- [ ] **G2 密钥不落树、不入日志。** 现状 `api_key` 可存 `/byteseek/llm/api_key`（会被 tree 观测到）。
  做：密钥仅走 env（对齐 commit `a315623` 的 env-only 方向），树里与日志里一律不出现明文。验收：
  `kvspace tree` 与日志中 grep 不到密钥。

---

## 建议推进顺序

1. **先地基**：A1+A2（恢复 / 持久）→ F1 最小回归（mock LLM），让后续改动有护栏。
2. **再扩能力**：B4（注册流程）→ B1/B2/B3（工具）→ B5（信任基线）。
3. **上定义能力**：C1→C3→C2（自造代码，先能造再能改）。
4. **补稳健与观测**：D1–D4、A3–A5、E1/E2 并行推进。
5. **收口**：G1/G2 安全基线 + F2 CI，对齐三条 exit criteria 后进入阶段二评审。
