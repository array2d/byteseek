# Livebyte 架构设计 v2

> livebyte = kvlang 执行引擎 + Claude Code 式 Agent Harness

## 定位

livebyte 不是一个新语言，也不是一个新 VM。**它是用 kvlang 编写的 Agent 程序**。
kvlang 提供执行引擎（call/return/br、vthread 调度、KV 状态持久化、tensor 计算），
livebyte 用这些原语组装出完整的 AI Agent。

```
+------------------------------------------------------------------+
|                       L I V E B Y T E                            |
|  (Agent 程序 — 用 kvlang 编写)                                     |
|                                                                  |
|  +------------+  +------------+  +------------+  +------------+  |
|  | agent_loop |  | tools/*.kv |  | compact.kv|  | team.kv    |  |
|  | (核心循环)  |  | (工具定义)  |  | (上下文压缩)|  | (团队协作)  |  |
|  +-----+------+  +-----+------+  +-----+------+  +-----+------+  |
|        |               |               |               |         |
+--------+---------------+-------+-------+-------+-------+---------+
                                 |
                                 v
+------------------------------------------------------------------+
|                       k v l a n g                                |
|  (KV 原生解释器)                                                   |
|                                                                  |
|  +----------+  +----------+  +----------+  +----------+         |
|  | kvcpu    |  | builtin  |  | layout   |  | dispatch |         |
|  | call/ret |  | arith/io |  | codegen  |  | cpu/metal|         |
|  | br/goto  |  | tensor   |  | vthread  |  | /cuda    |         |
|  +-----+----+  +-----+----+  +-----+----+  +-----+----+         |
|        |              |              |              |            |
+--------+--------------+------+-------+------+-------+------------+
                               |
                               v
+------------------------------------------------------------------+
|                    k v s p a c e                                 |
|  /src/*   /vthread/*   /data/*   /session/*   /tool/*   /lock/*  |
|  (Redis / 内存 / 文件 / etcd 任意后端实现 kvspace 接口即可)         |
+------------------------------------------------------------------+
```

## 核心原则

1. **livebyte 是 kvlang 程序，不是 Go 代码。** Agent 循环、工具调用、上下文压缩——全部用 `.kv` 文件表达。
2. **Agent 状态全在 kvspace。** kvlang 的 PC、栈帧、变量天然在 KV 路径中，Agent 的 session、messages、tools 同理。kvspace 是抽象 KV 存储接口，Redis 只是其中一个实现。
3. **LLM 调用是 kvlang 的一个 builtin op。** 就像 `add`、`print`、`tensor.new` 一样，`llm.call` 是一个内建算子。
4. **每个 Agent = 一个 vthread。** kvlang 的多 vthread 调度天然就是多 Agent 并行。

---

## 1. Agent Loop —— 用 kvlang 表达

Claude Code 的核心：

```python
while True:
    response = LLM(messages, tools)
    if stop_reason != "tool_use": return
    execute tools
    append results
```

livebyte 的 kvlang 表达：

```kvlang
# agent_loop.kv —— 核心 Agent 循环

def agent_loop() -> () {
    # 每个 session 有自己的 KV 子树：/session/{id}/messages
    # tools 定义在 /session/{id}/tools 中

    while (true) {
        # 1. 压缩检查
        estimate_tokens() -> './tokens'
        './tokens' > 100000 -> './need_compact'
        if ('./need_compact') {
            auto_compact() -> ()
        }

        # 2. 调用 LLM
        llm.call(
            "{{/session/current/model}}",
            "{{/session/current/system}}",
            "{{/session/current/messages}}",
            "{{/session/current/tools}}"
        ) -> './response'

        # 3. 检查停止原因
        './response' -> './stop_reason'

        # 4. 工具分发
        './response' -> './blocks'
        dispatch('./blocks') -> './results'

        # 5. 追加到 messages
        append_results('./results') -> ()

        # 6. TodoWrite nag
        check_todo_nag() -> ()
    }
}
```

**关键**：这不是伪代码。当 kvlang 的 builtin 正确注册 `llm.call`、`dispatch`、`estimate_tokens`
等算子后，这就是可直接 `./kvlang agent_loop.kv` 执行的程序。

---

## 2. 内建算子 —— livebyte 对 kvlang builtin 的扩展

kvlang 已有算术、比较、IO、tensor 等内建算子。livebyte 新增一批 **Agent 专用算子**。

### 2.1 LLM 调用

```
# 同步调用（阻塞 vthread 直到返回）
llm.call(model, system, messages, tools) -> response

# 流式调用（结果写入 pubsub channel）
llm.stream(model, system, messages, tools, channel) -> call_id

# Embedding
llm.embed(model, text) -> vector_path
```

### 2.2 工具系统

```
# 注册工具到 session
tool.register(name, schema, handler) -> ()

# 分发工具调用结果
dispatch(blocks, session_tools) -> results

# 执行单个工具
bash.run(command) -> output
file.read(path) -> content
file.write(path, content) -> ()
file.edit(path, old, new) -> ()
http.get(url) -> response
search.grep(pattern, path) -> matches
```

### 2.3 上下文管理

```
# Token 估算
estimate_tokens(messages) -> count

# 自动压缩（内部调用小模型做摘要）
auto_compact(session_id) -> ()

# 手动压缩
manual_compact(session_id, focus) -> summary
```

### 2.4 Todo 系统

```
# 更新 Todo 列表
todo.write(items) -> ()

# 检查是否需要提醒
check_todo_nag() -> need_nag
```

### 2.5 Session 管理

```
# 创建 session
session.create(model, system_prompt) -> session_id

# 加载 session
session.load(session_id) -> state

# 持久化 session 快照
session.save(session_id) -> ()
```

### 2.6 技能加载

```
# 按需加载 skill（类似 Claude Code 的 load_skill）
skill.load(name) -> content

# 列出可用 skills
skill.list() -> names
```

---

## 3. 工具系统 —— 基于 vthread 的 Worker 池

Claude Code 的工具分发是一个简单的 dict dispatch。livebyte 将其升级为
**vthread Worker 池**，利用 kvlang 的多 vthread 调度机制。

```
                    +------------------+
  主 Agent vthread  | tool: bash.run   |  写入 kvspace 队列
   (agent_loop)     | tool: file.write | ---->
                    | tool: llm.call   |
                    +------------------+
                            |
                            v
              +-------------+-------------+
              |    /tool/queue/{type}     |  kvspace LIST
              +-------------+-------------+
                            |
          +-----------------+-----------------+
          |                 |                 |
          v                 v                 v
   +-----------+     +-----------+     +-----------+
   | bash      |     | file      |     | llm       |
   | worker    |     | worker    |     | worker    |
   | vthread   |     | vthread   |     | vthread   |
   +-----------+     +-----------+     +-----------+
```

每个 worker 是一个**持久运行的 vthread**：

```kvlang
# bash_worker.kv
def bash_worker() -> () {
    while (true) {
        block_pop("/tool/queue/bash") -> './task'
        if ('./task' != "") {
            bash.run('./task.command') -> './output'
            kv.set('./task.result_key', './output')
        }
    }
}
```

优势：
- kvlang vthread 天然支持多 worker 并行调度
- 工具执行状态全在 kvspace 中，可观测、可恢复
- 可动态增减 worker 数量

---

## 4. Subagent —— vthread 就是子代理

Claude Code 的 subagent 是新进程 + 新上下文。livebyte 的 subagent 是**新 vthread**：

```
主 Agent 调用: spawn_subagent("分析目录结构", "explore")

  → kvspace.Set("/vthread/sub_001/...")  ← 创建新 vthread，PC="[0,0]"
  → kvspace.Notify("vm")                   ← 通知调度器
  → Worker Pick → Execute                  ← 子 vthread 开始执行
  → 子 vthread done                        ← 执行完毕
  → kvspace.Get("/vthread/sub_001/result") ← 读取结果
  → kvspace.Del("/vthread/sub_001/*")      ← 清理子 vthread
```

```kvlang
def spawn_subagent(prompt:string, agent_type:string) -> (summary:string) {
    # 创建子 vthread
    subagent_create(prompt, agent_type) -> './subagent_id'

    # 等待完成（block_pop 或 watch）
    subagent_wait('./subagent_id') -> ()

    # 读取结果
    subagent_result('./subagent_id') -> './summary'
}
```

与 Claude Code subagent 的对比：
- 相同：独立上下文、执行完毕返回摘要
- 不同：状态在 kvspace 中，断点可恢复、可并行多个、可观测执行进度

---

## 5. Agent Teams —— 多 vthread + 消息传递

```
+-------------------+    /team/inbox/alice    +-------------------+
| Alice (vthread-1) | <---------------------> | Bob   (vthread-2) |
| role: coder       |    /team/inbox/bob      | role: reviewer    |
+-------------------+                         +-------------------+
        |                                             |
        |          /team/inbox/lead                   |
        +--------------------+------------------------+
                             |
                  +-------------------+
                  | Lead (vthread-0)  |
                  | role: coordinator |
                  +-------------------+
```

```kvlang
def teammate_loop(name:string, role:string) -> () {
    # 工作阶段
    while (true) {
        # 检查 inbox
        check_inbox(name) -> './messages'
        if ('./messages' != "") {
            process_messages('./messages', name) -> ()
        }

        # 自动认领任务
        auto_claim_task(name) -> './task'
        if ('./task' != "") {
            execute_task('./task') -> ()
        }

        # 进入 idle 等待
        idle(60) -> ()
    }
}
```

**消息类型**（与 Claude Code 一致的 5 种）：
- `message` — 普通文本消息
- `broadcast` — 广播给所有队友
- `shutdown_request` / `shutdown_response` — 优雅关闭握手
- `plan_approval_response` — 计划审批

---

## 6. 上下文压缩 —— KV 原生的 compaction

kvlang 的 KV 寻址模型使上下文压缩极为自然——不是内存操作，是 KV 路径操作。

```kvlang
def auto_compact(session_id:string) -> () {
    # 1. 保存完整 transcript
    timestamp() -> './ts'
    transcript_save(session_id, './ts') -> ()

    # 2. 调用小模型做摘要
    llm.call(
        "claude-haiku",
        "Summarize this conversation for continuity.",
        "{{/session/{session_id}/messages}}",
        ""   # 不给工具，纯文本摘要
    ) -> './summary'

    # 3. 替换 messages（保持最近 2 轮）
    session.compact(session_id, './summary', keep_last=2) -> ()

    # 4. 注入身份快照（压缩后可能丢失身份上下文）
    session.inject_identity(session_id) -> ()
}
```

对比内存方案：
- Claude Code：`messages[:] = auto_compact(messages)` — 进程内存替换
- livebyte：`kv.Set("/session/{id}/messages", ...)` — KV 路径写入
- 优势：压缩结果持久化，restart 不丢，跨进程可见

---

## 7. Skill 系统 —— kvlang 的 on-demand 函数加载

```kvlang
# 系统启动时扫描 skills/ 目录
def register_skills() -> () {
    for_each("{{/skills/*/SKILL.md}}") {
        skill_parse(item) -> (name, desc, body)

        # Layer 1: 注入 system prompt（只注名称+描述）
        system_append("Skills: {name}: {desc}")

        # Layer 2: 按需加载（body 写入 KV）
        kv.set("/skill/{name}/body", body)
        kv.set("/skill/{name}/meta", desc)
    }
}

def load_skill(name:string) -> (content:string) {
    kv.get("/skill/{name}/body") -> './content'
}
```

---

## 8. 调度器 —— kvlang 内置的 LLM API 网关

kvlang 的 `dispatch` 包已有 CPU/Metal/CUDA 三后端分发。livebyte 新增 LLM 后端：

```
dispatch/
├── tensor/
│   ├── cpu/    ← 已有
│   ├── metal/  ← 已有
│   └── cuda/   ← 已有
└── llm/        ← 新增
    ├── anthropic.go
    ├── openai.go
    └── rate_limiter.go
```

调度器特性（全在 `livebyte:scheduler:*` kvspace key 中）：

| 机制 | kvspace 实现 |
|------|-------------|
| 优先级队列 | `livebyte:scheduler:queue` 有序集合，score=base_priority*1000 - wait_ms |
| 速率限制 | `livebyte:scheduler:rate_limit:{model}` HASH，滑动窗口计数 |
| 并发控制 | `livebyte:scheduler:inflight` SET，size 检查 |
| 重试/熔断 | 指数退避 + 连续 5 次错误暂停模型 30s |
| 流式推送 | SSE chunks → `livebyte:event:stream:{call_id}` PUBSUB |

---

## 9. 完整执行流程

```
用户输入: "分析这个项目的结构"
    │
    v
+-- livebyte CLI --+
| lb run analyze    |
+--------+----------+
         |
         v
+-- kvspace --+
| /session/{id}/messages  ← 追加 user message
+-----+-------+
      |
      v
+-- kvlang --+
| kvcpu worker picks vthread "agent_main"         |
|   PC = "[0,0]"                                  |
|                                                 |
| Execute:                                        |
|   [0,0] llm.call(model, system, msgs, tools)    |
|         → Scheduler: ZSET enqueue               |
|         → LLM Worker: Anthropic API call         |
|         → Response: tool_use: bash.run("ls")    |
|                                                 |
|   [1,0] dispatch(response.blocks)               |
|         → tool dispatch: bash-worker            |
|         → bash.run("ls") → list of files        |
|                                                 |
|   [2,0] append_result → /session/*/messages     |
|                                                 |
|   [3,0] br(stop_reason=="tool_use", [0,0], end) |
|         → loop back to [0,0] with new messages  |
|                                                 |
|   ... (多轮循环) ...                             |
|                                                 |
|   [N,0] return (stop_reason=="end_turn")        |
+-------------------------------------------------+
      |
      v
最终 text 输出 → 用户终端
```

---

## 10. 项目结构

```
livebyte/                          # Agent 程序（kvlang 源码）
├── agent/
│   ├── loop.kv                    # Agent 主循环
│   ├── tools.kv                   # 工具注册与分发
│   ├── compact.kv                 # 上下文压缩
│   ├── todo.kv                    # Todo 管理
│   ├── subagent.kv               # 子代理
│   └── team.kv                   # 团队协作
├── builtin/
│   ├── llm.kv                    # LLM 调用 builtin 注册
│   ├── session.kv                # Session 管理
│   └── skill.kv                  # Skill 加载
├── skills/                       # 内置 skills
│   ├── code-review/SKILL.md
│   ├── pdf/SKILL.md
│   └── test-writer/SKILL.md
├── bin/
│   ├── lb                         # 主 CLI
│   ├── lb-repl
│   └── lb-mon
└── session/                       # Session 数据（kvspace schema 定义）
    └── schema.kv

kvlang/                           # 执行引擎（Go 源码）
├── cmd/kvlang/                   # CLI 入口（已实现）
├── internal/
│   ├── kvcpu/                    # VM 执行循环（已实现）
│   ├── op/builtin/               # 内建算子（需新增 llm/session 等）
│   ├── op/dispatch/llm/          # LLM 后端分发（新增）
│   ├── layoutcode/               # AST → KV 布局（已实现）
│   └── kvspace/                  # KV 存储抽象接口（已实现，当前仅 redis 实现）
└── bin/kvlang                    # 编译产物
```

---

## 11. Claude Code vs livebyte v2

| 维度 | Claude Code | livebyte v2 |
|------|------------|-------------|
| 执行引擎 | Python 进程内 while 循环 | **kvlang VM**（call/return/br，vthread 调度） |
| 状态存储 | 进程内存（退出即丢） | **kvspace**（持久化，断点可恢复，后端可替换） |
| 工具分发 | Python dict dispatch | **vthread Worker 池** + kvspace LIST 队列 |
| 子代理 | subprocess 新进程 | **新 vthread**（共享 kvspace，天然可观测） |
| 上下文压缩 | 进程内 messages 替换 | **KV 路径操作**（持久化 + 跨进程可见） |
| 并发 | 单线程 + 子进程 | **多 vthread 并行**（Worker Pool + 调度器） |
| 编程模型 | 隐式（system prompt 引导） | **显式 kvlang 程序**（可测试、可组合、可版本化） |
| 张量计算 | 无 | **内置 tensor 生命周期 + GPU dispatch**（Metal/CUDA/CPU） |
| 可观察性 | stdout | **kvspace STREAM + 全体 KV 可查询** |
| 可扩展性 | 单机 | **分布式**（多 Worker 共享 kvspace，天然水平扩展） |

---

## 12. 一句总结

> **livebyte 不是"又一个 Agent 框架"。它是一台 KV 原生虚拟机上的 Agent 操作系统。
> kvlang 提供了 CPU（call/return/br）、内存（kvspace KV 路径）、进程调度（vthread）、
> 和 GPU 计算（tensor dispatch）。livebyte 在这之上编写 Agent 的"用户态程序"——
> agent loop、工具系统、上下文压缩、团队协作——全部是 `.kv` 文件，全部在 kvspace 中可观测。
> kvspace 是抽象接口，Redis 只是其中一个小规模实现。**
