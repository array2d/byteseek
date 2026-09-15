# byteseek —— KV 原生 agent substrate

> 实现：`lib/byteseek/*.kv` + `lib/local/config.kv`（执行逻辑、提示词、配置，全部 kvlang）。
> 无自有 Rust：byteseek 不是可执行文件，而是活在 kvspace 里的一棵 `.kv` 代码树，由标准
> `kvlang` 工具链（runtime-rs 编译出的 `kvlang` 二进制）驱动。

## 定位

agent 的「自己」——代码、状态、记忆、执行进度——全部活在**同一棵可寻址、可持久、可自改的
KV 树**（kvspace，后端 redis/fs/shm/s3）里。LLM、shell、python、json、http 通过 rwir 成为
这棵树里的一等公民。byteseek 自身不叠加任何宿主语言：它复用 kvlang 标准 rwir，主脑逻辑全是
`lib/` 下的 kv rwfunc。

## 源码结构

```
lib/byteseek/main.kv         main / mainbrain（REPL 循环）/ run（动态执行生成程序）
lib/byteseek/llm.kv          llm·call(userinput) -> entry：代码脑，LLM 生成 kv 程序并入库
lib/byteseek/prompt.kv       系统提示（lib prompt 顶层写 → byteseek/prompt·init，layout 期种入）
lib/byteseek/memory.kv       记忆：remember / recall / classify（类别名跟随消息语言）/ lang
lib/byteseek/memgen.kv       能力记忆：把 /lib/<name> 浓缩成 ·mem 条目
lib/byteseek/tools.kv        工具清单：layout 期把工具声明种进 /byteseek/tools/，manifest() 渲染
lib/local/config.kv          LLM 配置（lib local 顶层写 → local·init，layout 期种入；gitignored）
vendor: kvlang stdlib        工具能力全在 stdlib——networld/shell·run / networld/python·run / networld/*
```

## 引导与运行

byteseek 无「拉起进程」一说。两步：

```bash
KVLANG_LIB=lib kvlang        # 引导：layout lib/ 全部 .kv 进 kvspace，并执行各 init
kvlang byteseek·main         # 运行：驱动已入库的 funckey（进入 REPL）
```

`KVLANG_LIB=lib kvlang`（无 entry）复用标准 kvlang 的「layout 全部 lib + 跑各 init」机制：
`config.kv`/`prompt.kv` 的顶层写语句被 parser 合成为 `local·init` / `byteseek/prompt·init`，
在 layout 期执行一次，把配置、系统提示种进 kvspace（跑完即删 init 子树）。kvlang 语法速览
（`kvlangbrief`）由 **kvlang 自己的 stdlib**（`kvlang/stdlib/kvlangbrief.kv`，lib kvlang）在
runtime-rs 启动时种入 `/lib/kvlang/kvlangbrief`，不属 byteseek lib。rwfunc（`main`/`mainbrain`/
`run`/`llm·call`/`networld/shell·run`/`networld/python·run`）留在 `/lib` 下持久。之后 `kvlang byteseek·main`
直接驱动 funckey，不再 layout、不再重跑 init。`byteseek·main` 是 funckey 路径（去 `/lib`
前缀），不是文件路径。

## 状态树布局

```
/byteseek/llm.api            LLM 接口地址（config.kv 种入）
/byteseek/llm.key            LLM 鉴权 key（config.kv 种入，可空）
/byteseek/llm.print          是否打印 LLM 交互（config.kv 种入）
/byteseek/prompt/system      系统提示（prompt.kv 种入）
/lib/kvlang/kvlangbrief    kvlang 语法速览（kvlangbrief.kv 种入；llm·call 拼进 system prompt）
/lib/byteseek/session/<name> llm·call 生成并入库的一次性程序（可寻址/持久）
/vthread/{vid}/‥pc           执行到哪一步（KV 路径字符串，崩溃可恢复）
/lib/<pkg>·<name>/...        编译后函数（签名 + 指令 + 源码 .src）
```

## 提示词与 LLM 参数也是 KV 数据

系统提示不硬编码，而是 `prompt.kv` 的 `lib prompt` 顶层写把提示词落进 `/byteseek/prompt/system`；
语法速览由 `kvlangbrief.kv` 落进 `/lib/kvlang/kvlangbrief`。`llm·call` 每轮把两者拼成
system prompt。接口地址与 key 由 `config.kv` 种入。换模型/网关/提示词只需改树，无需重编译——
本就无可编译之物。

## 代码脑：llm·call 生成 kv 代码（自造代码）

主循环不是「LLM 决定动作类型 → 分派固定工具」，而是 **LLM 直接生成一段 kvlang 程序并执行**：

```kv
rwfunc main() -> () { byteseek·mainbrain() }
rwfunc mainbrain() -> () {
    running = 1
    while (running == 1) {
        input("byteseek> ") -> userinput
        string·cmp(userinput, "exit") -> q
        if (q == 0) { running <- 0 }
        else { llm·call(userinput) -> entry; byteseek·run(entry) }
    }
    println("bye.")
}
```

`llm·call(userinput)`：system prompt（提示词 + 语法速览）→ LLM → 解析 `<name>`/`<kv>` →
包进 `lib byteseek { lib session { lib NAME { … NAME·main() } } }` → `kvlang·vet` 校验 →
通过后 `kvlang·layout` 入库 → 返回入口 `byteseek/session/NAME·init`。生成失败回填
`error: …`，`byteseek·run` 据前缀跳过执行。

## byteseek·run：同 vthread 动态执行（进程↔vid 1:1）

```kv
rwfunc run(entry:[]char/utf32) -> () {
    if (entry == "") { return }
    string·find(entry, "error") -> iserr
    if (iserr == 0) { println("[byteseek] 跳过执行（生成失败）:", entry); return }
    vthread·call(entry)
}
```

`vthread·call(funckey)` 是 kvlang 的 native builtin（`runtime/src/builtin.c`）：在**当前 vthread**
（同 vid）按运行时 funckey 造一次动态 `OP_CALL`，跑到被调函数结束再回到本指令的 NextPc。与
`vthread·run` 不同——不新开 vid、不 WATCH 挂起——被调程序里的 rwir（`println`/`networld/shell·run`/…）
由当前驱动就地派发。故一次 REPL 请求、生成程序的执行、其内工具调用全在同一进程同一 vid 内完成，
契合「一个进程 ⟺ 一条 vthread」。

## rwir：全部复用 kvlang 标准集

byteseek 不再注册任何自有 Rust rwir。所需能力全部是 kvlang 标准 rwir（runtime-rs / runtime-c）：

| opcode | 来源 |
|--------|------|
| `print` / `println` / `cerr` / `input` | 标准 term rwir |
| `json·to` / `json·from` | 标准 json rwir |
| `http·call(method,header,url,body) -> resp` | 标准 http rwir |
| `kvlang·vet / ·format / ·layout / ·dump` | 标准 layout rwir |
| `networld/proc·exec(args,envs) -> code, out, err` | 标准 networld rwir（子进程 + 捕获 @ 句柄） |
| `vthread·call(funckey)` | native builtin（同 vid 动态调用） |
| `string·* / kv·* / xv·*` | native builtin |

`networld/shell·run` / `networld/python·run` 是 `lib/` 下的 kv rwfunc：把命令包成 `{"bash","-c",cmd}` /
`{"python3","-c",code}` 交给 `networld/proc·exec`，绑定 stdout/stderr 写槽即捕获（`@[]uint8`
扩展句柄，`println` 读时按 body 前缀 `/networld/{host}/proc` 路由回兑现物理字节），返回 stdout。

## 文件编辑：networld/edit（kvlang stdlib）

`networld/edit` 是 kvlang stdlib（`kvlang/stdlib/networld/edit.kv`，tutorial
`14-networld/12-edit.kv`）里 `networld/fs`（字节原语 + 只读检索）之上的**改写语义层**：

| 工具 | 语义 |
|------|------|
| `networld/edit·read(p, from, lines)` | 行号窗口视图（给模型看，不是全文 dump），返回 (视图, 总行数, 字节数) |
| `networld/edit·write(p, t)` | UTF-8 覆盖写 + 回读校验 |
| `networld/edit·replace(p, old, new, dry)` | 单处精确替换：`old` 必须**恰命中 1 处**，否则一个字节都不动 |
| `networld/edit·multi(p, olds, news, dry)` | 事务式多处替换：全部成功才落盘 |

返回码统一：`0` 成功 / `1` 未找到 / `2` 歧义（命中多处）/ `3` 不可读 / `4` 写失败 / `5` 回读不一致 /
`6` 两数组长度不等。库不产出话术——「怎么说给模型听」属 harness 策略。

## 工具调用边界（issue #72）

pi 的 toolResult 边界在 byteseek 里落到 KV 树上：

| 环节 | 落点 | 说明 |
|------|------|------|
| 工具清单 | `/byteseek/tools/*` + `byteseek/tools·manifest()` | 树数据；`llm·call` 拼 system prompt 时注入《可用工具》段，新增工具 = 写一条 |
| 执行 | `byteseek·run(entry) -> (kind, detail)` | `vthread·create` + `vthread·run` 跑在**子 vid**，父（会话）不死 |
| 记账 | `/byteseek/tool/<n> = "kind \| detail \| entry"` | 可寻址、可复盘；detail 截断到 300 字 |
| 用量 | `/byteseek/usage/<n>` + `usage/last` = `prompt=… completion=… total=…` | 每次 LLM 调用记一次（`llm·usage()`） |
| 程序自报 | `/byteseek/last/result` = `ok` / `fail: 原因` | 工具级失败（如 `edit·replace` code != 0）由程序按协议上报 |
| 回灌 | `byteseek·solve(prompt)` | 失败把 `[上轮执行未成功] kind：detail` 拼回 prompt，有界重试 ≤3 轮 |

kind 分类：`truncated`（被 max_tokens 截断）/ `vet`（生成物不过闸）/ `runtime`（子 vthread 报错，
读 `/vthread/<vid>/‥error/msg`）/ `fail`（程序自报失败）/ `ok` / `empty`。

配套 runtime 语义：`vthread·run(vid)` 下**子 vthread 自己失败不再冒泡杀死父**——监督者活着读
`/vthread/<vid>/‥status` 与 `‥error/msg`（一个子任务失败不该结束整条会话）。

落盘是原子的：`networld/edit·write/replace/multi` 都走 `networld/edit·commit` —— 写 `<p>.tmp`
→ `networld/fs·rename` 覆盖 p → 回读校验；失败时 p 保持原样，不留半截文件。

## 已验证（0rs）

- `KVLANG_LIB=lib kvlang` 引导：config/语法速览/系统提示三 init 于 layout 期种入，rwfunc 持久。
- `kvlang byteseek·main`：REPL 循环、`exit` 退出、无 key 时 `llm·call` 回填 error 且 `byteseek·run` 跳过。
- `byteseek·run(entry)` → `vthread·call`：同 vid 动态执行入库的 session 程序，其内 rwir 就地派发。
- `networld/shell·run` / `networld/python·run`：经 `networld/proc·exec` 捕获子进程 stdout 并透明兑现。
- `make test`（无网络无 LLM，shm 后端）：vet + session 执行 + shell/python 捕获全链路通过。
