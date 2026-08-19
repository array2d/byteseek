# byteseek —— KV 原生 agent substrate

> 日期：2026-08-19
> 实现：`src/`（corebrain 引擎，Rust）+ `lib/byteseek/*.kv`（执行逻辑与提示词）

## 源码结构

```
src/main.rs        入口：连 kvspace → 清空 → 连 runtime → 注册 rwir → 种 LLM 配置
                   → layout 全部内嵌 lib/byteseek/*.kv → 跑 byteseek.init → 进 REPL
src/ffi.rs         C ABI：kvspace-durable（自持句柄）+ kvlang runtime（模式2 主导执行）
                   + rwirext 宿主 ABI + kvlang layout（vet/layout/format）
src/engine.rs      Engine：kvspace 读写、talk 队列、主导驱动 vthread（run_fn 可重入）
src/rwir/          一等公民 rwir，一模块一职责：
  ├─ io.rs         print / println / cerr / input（term 输入输出）
  ├─ llm.rs        llm.call(userinput) -> entry：代码脑，LLM 生成 kv 程序
  ├─ json.rs       json.to / json.from：KV 子树 ↔ JSON（rust 内嵌，照搬 go/json）
  ├─ http.rs       http.call(method,header,url,body) -> resp（rust 原生 ureq）
  ├─ kvlayout.rs   kvlanglayout.vet / .layout / .src（layout C ABI）
  └─ mod.rs        注册表 REGS + dispatch 派发 + shell/python 共用 tool_run
```

## 定位

byteseek 不是又一个 agent 框架。agent 的「自己」——代码、状态、记忆、执行进度——全部
活在**同一棵可寻址、可持久、可自改的 KV 树**（kvspace，后端 redis/fs/s3）里。LLM、
shell、python、json、http 通过注册 rwir 成为这棵树里的一等公民。

一个进程 = 一个 **corebrain**：把 `.kv` 布局进 kvspace → 注册 rwir → bootstrap 一条
vthread → 主导驱动执行（kvlang 模式 2），遇 rwir 就地处理。扩展库（json/http/…）是
**单进程内嵌**的 Rust rwir + `lib/` 下 kv 源码的 rwfunc，不再起独立进程。

## 状态树布局

```
/byteseek/llm.api            LLM 接口地址（seed 从 DEEPSEEK_API_URL 读入，有默认）
/byteseek/llm.key            LLM 鉴权 key（DEEPSEEK_API_KEY，可空）
/byteseek/prompt/system      系统提示（prompt.kv 的 seed_prompts() 播种）
/lib/byteseek.kvlangbrief    kvlang 语法速览（llm.call 拼进 system prompt）
/byteseek/session/talk/*     对话队列（input 落入，可寻址/持久）
/vthread/{vid}/‥pc           执行到哪一步（KV 路径字符串，崩溃可恢复）
/lib/<pkg>.<name>/...        编译后函数（签名 + 指令 + 源码 .src）
```

一个 session 的全部状态可直接观测：`kvspace tree /byteseek/session/`。

## 提示词与 LLM 参数也是 KV 数据

系统提示不硬编码在 Rust 里，而是 `lib/byteseek/prompt.kv` 的 `seed_prompts()` 把提示词
写进 `/byteseek/prompt/system`；语法速览由 `kvlangbrief.kv` 落进 `/lib/byteseek.kvlangbrief`。
`llm.call` 每轮把两者拼成 system prompt。LLM 接口地址与 key 由 `llm::seed` 从环境读入
`/byteseek/llm.api` / `/byteseek/llm.key`。换模型/网关/提示词只需改树，无需重编译。

## 代码脑：llm.call 生成 kv 代码（自造代码）

corebrain 的主循环不再是「LLM 决定动作类型 → 分派固定工具」，而是 **LLM 直接生成一段
kvlang 程序并执行**：

```
rwfunc main() -> () { seed_prompts(); mainbrain() }
rwfunc mainbrain() -> () {
    running = 1
    while (running == 1) {
        input("byteseek> ") -> userinput
        string.cmp(userinput, "exit") -> q
        if (q == 0) { running <- 0 }
        else { llm.call(userinput) -> entry; byteseek.run(entry) }
    }
    println("bye.")
}
```

`llm.call(userinput)`：system prompt（提示词 + 语法速览）→ LLM → 解析 `<name>`/`<kv>` →
包进 `lib byteseek { lib session { lib NAME {…} } }` → `kvlanglayout.vet` 校验 → 通过后
`kvlanglayout.src` layout 入库 → 返回入口 `byteseek/session/NAME.init`。生成失败回填
`error: …`，`byteseek.run` 跳过执行。

## rwir（一等公民）

在 `rwir::register`（`src/rwir/mod.rs` 的 `REGS` 表）里注册到 `/lib/<opcode>`：

| opcode | 处理 |
|--------|------|
| `print / println / cerr / input` | term 输入输出（io.rs） |
| `llm.call(userinput) -> entry` | 代码脑：LLM 生成 kv 程序入库 |
| `byteseek.run(entry)` | 把生成的 kv 程序作为嵌套 vthread 跑完 |
| `shell.run(cmd) -> out` / `python.run(code) -> out` | 子进程工具（mod.rs tool_run，截断 TOOL_CAP） |
| `json.to(root) -> str` / `json.from(str) -> root` | KV 子树 ↔ JSON |
| `http.call(method,header,url,body) -> resp` | 网络抓取（rust 原生 ureq） |
| `kvlanglayout.vet / .layout / .src` | .kv 校验/入库（layout C ABI） |

`lib/byteseek/http.kv` 是 rwfunc 标准库示例：`lib http { rwfunc get/post/put/del }` 封装
`http.call`（kv 源码，可寻址/可自改）。

## 模式 2 主导驱动

`Engine::run_fn(funcname)`：`kvlangRuntimeBootstrap` 拿 vid → 循环
`kvlangRuntimeExecuteVthread`：rc==0 结束；rc==1 遇 ext rwir，就地 `dispatch`（op 由
`kvlang_rwirextParams` 首行解码），再 `kvlang_rwirextNextPc` 写回 `/vthread/{vid}/‥pc`。
可重入：`byteseek.run` 在 dispatch 里嵌套调用 `run_fn`。

## 实现要点（踩坑记录）

**带 scope 的函数必须在被调用的嵌套帧里执行。** layout 禁止顶层 `while`，会把顶层语句
包进 `init` 帧。若直接 bootstrap 带 scope 的函数作为顶层 vthread 帧，其 while 的 scope PC
无法经该帧的 extindex 解析，会静默失败。正确做法：`main()` 无 scope，由它调用带 while
的 `mainbrain()`，让 scope PC 在嵌套调用帧里正确寻址。见 `run_fn`。

## 运行

```bash
cargo build --release
KVSPACE=redis://127.0.0.1:6379 DEEPSEEK_API_KEY=... ./target/release/byteseek
```

环境变量：`KVSPACE`（默认 `redis://127.0.0.1:6379`）、`DEEPSEEK_API_URL`（默认 deepseek）、
`DEEPSEEK_API_KEY`。

## 已验证

- json.to/from 子树 ↔ JSON 往返（fs 后端单测，输出与 tutorial/10 一致）
- http lib 源码 vet（`[]char/utf32` 参数类型）
- 多级嵌套 lib layout + 增量合并（layout 单测，`lib a { lib b {…} }`）
- layout 三 ABI：vet / format（保 lib 分组、幂等）/ layoutcode（按函数覆盖）
