# JIT-Agent 参考 —— 即时进化的 agent harness（arXiv 2608.25593）

> 日期：2026-08-31
> 来源：`reference/deepx-code` 旁的 `reference/JIT`（bingreeky/JIT，git@github.com:bingreeky/JIT.git）
> 用途：byteseek 的「llm.kv 自举（模型生成 kv agent）」与「rwir/rwfunc 版本+评分进化（#7）」、
> 「agent 核心循环四层（#8）」、「记忆系统（#9/#10/#11）」的对照蓝本。

---

## 一、项目是什么

**JIT-Agent** —— 「**即时进化 agent harness**」的元 agent。不预编译通用脚手架，
而是根据 **任务描述 + 协议 + 工具/skill 注册表 + 检索到的历史 harness**，现场生成
**任务特化的可执行 harness**，包装任意商用 agentic LLM（**Model-as-a-Harness**）。

核心循环（`jit/meta_agent.py`）：`generate → validate → repair(retry)` 确定性循环
+ **review panel（多专家投票 pass/fail）**。跑完拿到 trace/feedback → 修复 harness →
更新档案库。**harness 在测试时持续进化，生成器冻结**。

> 结论先行：**JIT 是「harness 即产物、可进化」的范式，byteseek 的 llm.kv 自举正是
> 同一条路。** JIT 给了这套范式的完整工程实现，byteseek 可把它当「从 kvlang 视角重写」
> 的蓝本。

## 二、架构：四模块 harness 因子化

每个 harness 固定 4 个 Python 模块 + YAML 配置（`harness_factory/harnesses/<名>/`）：

| 模块 | 职责 |
|------|------|
| `memory.py` | 上下文 / 状态管理 |
| `planning.py` | 规划 / 策略更新 |
| `action.py` | 控制 agent 执行流 |
| `tool_policy.py` | 工具暴露与使用边界 |

生成 prompt 强约束：生成代码必须**继承参考 harness 的核心机制**（如具体的内存压缩策略、
子 agent 任务分解），不能抽象成模糊概念自由发挥。生成结果是**结构化代码**而非自由式
agent 程序（`harness_ops.py` 解析模型的 5 个 tagged block）。

## 三、种子 harness 参考库（11 个）

`_pick_reference_harnesses` 随机挑 k 个给模型当参考材料。设计 write-up 在
`harness_factory/descriptions/*.md`。

| Harness | 核心思想 |
|---|---|
| `plan_and_execute` | 线性 ReAct。前置 3–7 步路线图再执行（最小基线） |
| `flash_searcher` | planning 把任务分解成 DAG 子任务，按依赖序执行 |
| `agentfold` | DAG 规划 + AgentFold 式上下文折叠，长轨迹留在窗口内 |
| `resum` | 线性 ReAct + ReSum 式 token 预算轨迹摘要 |
| `hiagent` | 扁平 ReAct，无显式计划，靠结构化工作记忆 |
| `memobrain` | marker 式 ReAct + 依赖感知推理图记忆（token 超窗 LLM 压缩） |
| `deepagent` | 扁平无计划工具使用，marker 协议非 JSON 工具调用 |
| `gam` | DAG 规划 + 生成式 agent 记忆 |
| `roma` | 递归分解（若干半独立目标的任务） |
| `aggagent` | 两阶段：先广探索，再裁决成答案 |
| `oagent` | 多条独立解路径并行投票 |

## 四、对 byteseek 的参考价值（逐条对应）

| byteseek 方向 | JIT 对应物 |
|------|------|
| **#7** rwir/rwfunc 版本+评分进化（n 变体实测择优，沉淀通用/本地最优库） | `jit/selector.py` **best-of-N**（logprob / judge 双路）+ 档案库更新 |
| **#8** agent 核心循环 session/task/turn/thread 四层 | **四模块 harness 因子化**（memory/planning/action/tool_policy）——直接的组织模板 |
| **#9/#10/#11** 记忆系统（会话外两阶段、读路径薄注入、合并子代理并发安全） | `memory.py` 各策略：`memobrain` 依赖感知推理图 + token 超窗 LLM 压缩；`resum` token-budget 摘要；`agentfold` 上下文折叠 |
| **llm.kv 自举**（模型生成 kv agent） | **generate→validate→repair 循环 + review panel**——「测试时进化」的现成实现模板 |
| **kvlangbrief**（语法速览） | 可叠加 **harness 架构参考库**：像 JIT 把种子 harness 描述塞进生成 prompt，byteseek 可把成熟 kv agent 结构（记忆/规划/动作）当参考给模型 |
| 工具边界 | `tool_policy.py` 模块思路（工具暴露与使用边界） |

## 五、最值得挖的文件

- `jit/prompt.yaml` —— 生成 / 修复 / 审查 prompt。含「继承参考机制而非抽象」的强约束、
  四模块 + YAML 配置的 tagged-block 输出格式、review panel 投票协议。
- `jit/meta_agent.py` —— generate→validate→repair 循环 + repair_history + 超时看门狗。
- `jit/selector.py` —— best-of-N（logprob / judge）。
- `jit/schemas.py` / `jit/harness_ops.py` —— 请求/结果/校验记录 dataclass、5 个 tagged block 解析。
- `harness_factory/harnesses/*/memory.py` —— 多种记忆压缩策略（memobrain 推理图、resum 摘要）。
- `harness_factory/descriptions/*.md` —— 11 个 harness 的设计 write-up（也作生成 prompt 参考材料）。

## 六、落地建议（byteseek）

1. **把 JIT 四模块因子化翻译成 kvlang**：`lib/byteseek/` 下的记忆/规划/动作/工具边界
   各成一个 `.kv` 库，`llm·call` 生成时按四模块组装，而非自由生成。
2. **补 best-of-N 择优**：#7 的「n 变体实测择优」可直接借鉴 `selector.py`——生成 n 个
   候选 harness，跑完按 logprob 或 judge 评分，沉淀最优到 `/lib/byteseek/session/` 档案库。
3. **generate→validate→repair 循环**：byteseek 已有 `kvlanglayout·vet` 闸门，可扩成
   JIT 式完整循环（生成 → vet/试跑 → 反馈 → repair）。
4. **记忆模块对照**：#9/#10/#11 的实现可对照 `memobrain`（依赖感知推理图 + token 预算压缩）
   与 `resum`（摘要），选定一种在 kvspace 里落地。
