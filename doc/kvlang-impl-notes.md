# livebyte × kvlang 实现笔记

> 作者：Claude（kvlang 当前最熟悉的维护者）  
> 日期：2026-07-15  
> 写给：livebyte 的第一个实现者

---

## 一、哪些东西在 kvlang 里实际能跑

architecture.md 的方向正确，但有几处伪代码写法在当前 kvlang 里无法直接执行。
区分清楚哪些能跑、哪些需要新增，能节省大量试错时间。

### ✅ 已经能跑的

| 写法 | 说明 |
|------|------|
| `while (true) { ... }` | lower 展开为 `br` 块 + TCO，无栈溢出 |
| `if (cond) { ... }` | 正常 |
| `add(a, b) -> ./r` | 函数调用 + 写槽 |
| `kv.Watch`（BLPOP） | 已在 kvspace 接口实现 |
| `kv.Notify`（LPUSH） | 已实现 |
| 多 vthread 并发调度 | kvcpu worker pool + pick/wait 已实现 |

### ❌ 当前不存在、需要新增

**1. `"{{/path/key}}"` 模板插值**

```kv
# architecture.md 里写的：
llm.call("{{/session/current/model}}", ...)

# kvlang 里没有这个语法。正确写法：
kv.get("/session/current/model") -> "./model"
llm.submit("./model", "./session_id") -> "./call_id"
```

模板插值需要 scanner 层改动，且与 kvlang "key=路径, value=标量" 的原则相冲突——
路径是运行时值，不是字符串字面量的一部分。**建议永远不加**，保持显式读取。

**2. `for_each("{{/skills/*/SKILL.md}}")`**

kvlang 的 `for` 迭代原语尚未实现（lower/todo.md P11）。
当前替代：实现一个 `skill.list() -> count` + 显式索引迭代的 builtin，
或等 `for` lowering 完成后改写。

**3. `'./task.result_key'` 路径间接引用**

```kv
# architecture.md 里写的：
kv.set('./task.result_key', './output')

# 这不是合法写法。kv.set 的第一个参数是 key 字符串，不是"存着 key 的槽"。
# 正确：先读出目标路径，再写入
kv.get("./task") -> "./task_key"
kv.set("./task_key", "./output")  # 仍需要 builtin 支持动态 key
```

动态 key（key 是运行时值而非字面量）需要专门的 `kv.setat(key_slot, val_slot)` builtin。

---

## 二、最关键的实现决策：`llm.call` 的异步模型

这是 livebyte 能否跑起来的核心。

**错误模型**（同步阻塞）：
```
vthread → llm.call → 等待 30 秒 → 返回
```
kvlang 的 kvcpu worker 在这 30 秒内被占用，无法调度其他 vthread。
8 个 worker + 8 个 LLM 调用 = 系统卡死。

**正确模型**（Watch/Notify 异步）：
```
vthread → llm.submit → 立即返回 call_id
vthread → llm.await(call_id) → kv.Watch → 释放 worker goroutine
                                     ↑
LLM HTTP worker (独立 goroutine) → 写结果 → kv.Notify → vthread 被唤醒
```

kvlang 的 `Watch` 底层是 Redis `BLPOP`，**天然就是这个语义**。
`llm.submit` 把请求放进队列并返回，`llm.await` 阻塞在结果 key 上。
这是 kvlang 设计里最适配 AI agent 的地方——请按此实现。

```kv
def agent_loop() -> () {
    while (true) {
        llm.submit("./session_id") -> "./call_id"
        llm.await("./call_id", 120) -> "./response"  # 120s timeout
        dispatch("./response", "./session_id") -> ()
        stop_reason("./response") -> "./sr"
        br("./sr" == "end_turn", end, continue_label)
    }
}
```

---

## 三、消息列表的 KV 存储方案

LLM messages 是结构化列表，但 kvlang 的 Value 是标量。

**不推荐**：把整个 messages JSON 存在一个 key 里。
这违反 kvlang "value = scalar" 的设计原则，读写都需要 JSON 编解码，性能差，
且无法用 `kv.List` 检视消息树。

**推荐**：路径枚举，内容追加时只写新增的 key：

```
/session/{id}/msg/count          = "N"
/session/{id}/msg/{n}/role       = "user" | "assistant"
/session/{id}/msg/{n}/text       = "..."
/session/{id}/msg/{n}/tool_calls = "..."   # 可选，JSON 或再展开
```

写新消息只需：
```kv
msg.append("./session_id", "user", "./user_text") -> ()
# 实现：incr count，写 role + text 到对应编号
```

发 LLM 请求时，由 `llm.submit` builtin 内部把这个 KV 树序列化成 API 需要的 JSON——
这个序列化逻辑在 Go 里写，kvlang 代码完全不感知。

---

## 四、builtin 注册优先级

按依赖顺序，最小可用集：

```
Phase 0 — kvlang 引擎层已有
  kv.get / kv.set / kv.watch / kv.notify / kv.list / kv.del

Phase 1 — 最先实现，其他一切依赖它
  llm.submit(session_id) -> call_id      # enqueue LLM request
  llm.await(call_id, timeout) -> result  # BLPOP on result key
  msg.append(session_id, role, text)     # 写入消息树
  msg.count(session_id) -> n             # 读消息数量

Phase 2 — agent_loop 跑起来需要
  session.create(model, system) -> session_id
  session.load(session_id) -> ()
  dispatch(response, session_id) -> ()   # 解析 tool_use blocks，写队列
  stop_reason(response) -> reason

Phase 3 — 工具执行
  bash.run(cmd) -> output
  file.read(path) -> content
  file.write(path, content)
  file.edit(path, old, new)

Phase 4 — 高阶特性
  skill.load / skill.list
  session.compact
  subagent.spawn / subagent.await
```

---

## 五、入口约定：`init()` 而非 `pre_main`

architecture.md 提到 `lb run analyze` → 执行 `agent_main`。
kvlang 正在迁移到 `def init()` 作为唯一隐式入口约定（parser/todo.md S9）：

- `agent_loop.kv` 中无需 `def main()`，顶层语句自动进 `init()`
- `kvlang load agent_loop.kv` → 注册函数，入口为 `agent_loop/init`
- 或直接 `kvlang agent_loop.kv` → 解析 + 执行 `init()`

`lb` CLI 可以直接包装 `kvlang`，不需要重新实现加载逻辑：

```bash
lb run <file.kv>     ≡     kvlang <file.kv>
lb agent <name>      ≡     kvlang --entry <name>/init
```

---

## 六、subagent = vthread，正确的姿势

architecture.md 的 subagent 模型是对的，补充实现细节：

```kv
def spawn_subagent(prompt:string, agent_type:string) -> (result:string) {
    # 1. 在 kvspace 准备子 agent 的输入
    session.create("claude-3-5-sonnet", "./system") -> "./sub_id"
    msg.append("./sub_id", "user", "./prompt")

    # 2. 提交 vthread（Bootstrap 创建 /_fn 链接）
    vthread.spawn("agent_loop", "./sub_id") -> "./vtid"

    # 3. 等待完成（Watch .status key）
    vthread.await("./vtid", 300) -> "./result"
}
```

`vthread.spawn(funcName, arg)` builtin：
- 在 Go 层调用 `layoutcode.Bootstrap` + `vthread.Set` + `kv.Notify`
- 完全复用已有的 kvcpu 调度路径
- **不需要 subprocess，不需要 goroutine，kvcpu worker pool 自动处理**

这是 kvlang 相对 Claude Code 最大的结构性优势——subagent 的开销是一次 KV 写入，
而不是一个新进程。

---

## 七、团队协作：收件箱就是 KV 路径

architecture.md 的团队消息总线用 kvspace 实现天然正确，
但注意与 `kv.Notify`（LPUSH + BLPOP）的对应：

```
/team/{team_id}/inbox/{agent_name}   ← Notify 写入，Watch 消费
```

消费端：
```kv
def teammate_loop(name:string) -> () {
    while (true) {
        kv.watch("/team/main/inbox/" + name, 60) -> "./msg"
        if ("./msg" != "") {
            process_message("./msg") -> ()
        }
        auto_claim_task(name) -> "./task"
        if ("./task" != "") { execute_task("./task") -> () }
    }
}
```

`kv.Watch` 的 60 秒超时 + 循环 = 既能响应消息，又能主动认领任务，
不需要额外的 `idle(60)` 机制——kvlang 的 Watch 本身就是有超时的阻塞等待。

---

## 八、最容易踩的坑

1. **`./path` 是帧局部变量，不是 KV 绝对路径**  
   `kv.get("./session_id")` 读的是当前帧的局部变量 `session_id` 的值，
   而不是 `/session_id`。
   绝对路径用字符串字面量：`kv.get("/session/42/model")`，
   或先读到临时变量再拼接。

2. **vthread 的 `.status` 在终态时 Del+Notify，不是 Set**  
   `vthread.await` 应该 Watch `.status` key，收到通知后读结果，
   不要轮询。

3. **kvcpu worker 数量 ≠ 并发 LLM 请求数量**  
   Worker 是执行指令的 goroutine，LLM HTTP 请求应该在独立 goroutine pool 里跑。
   `llm.submit` 写队列，独立的 LLM worker goroutine 消费队列、调 HTTP、写结果、Notify。
   两个 pool 完全解耦。

4. **`kv.List` 返回直接子节点，不递归**  
   `/session/42/msg` 的 List 返回 `["count","0","1","2"]`，
   不会深入 `/session/42/msg/0/role`。需要两层 List 才能遍历消息树。

5. **字符串拼接目前没有内建算子**  
   `"prefix" + str_var` 还不存在。需要实现 `string.concat` builtin 或
   利用格式化 builtin。这是 Phase 1 就要解决的基础设施。

---

## 九、推荐的第一个里程碑

**目标**：能跑通一次最简单的 LLM 对话

```kv
# hello_agent.kv
session.create("claude-3-5-sonnet", "You are a helpful assistant.") -> "./sid"
msg.append("./sid", "user", "say hello") -> ()
llm.submit("./sid") -> "./cid"
llm.await("./cid", 30) -> "./resp"
msg.last_text("./resp") -> "./answer"
print("./answer")
```

这条路径涵盖：session 创建、消息写入、LLM 异步调用、结果读取、输出。
全程走 kvlang 的标准执行路径，`init()` 入口，无任何特殊处理。

**跑通这个，livebyte 的核心闭环就建立了。**
之后的 while 循环 + tool dispatch + subagent 都是在这条路径上的增量扩展。
