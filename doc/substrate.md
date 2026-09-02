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
lib/byteseek/kvlangbrief.kv  kvlang 语法速览（顶层写 → byteseek·init，layout 期种入）
lib/byteseek/shell.kv        shell·run(cmd) -> out：bash 子进程，捕获 stdout
lib/byteseek/python.kv       python·run(code) -> out：python3 子进程，捕获 stdout
lib/local/config.kv          LLM 配置（lib local 顶层写 → local·init，layout 期种入；gitignored）
```

## 引导与运行

byteseek 无「拉起进程」一说。两步：

```bash
KVLANG_LIB=lib kvlang        # 引导：layout lib/ 全部 .kv 进 kvspace，并执行各 init
kvlang byteseek·main         # 运行：驱动已入库的 funckey（进入 REPL）
```

`KVLANG_LIB=lib kvlang`（无 entry）复用标准 kvlang 的「layout 全部 lib + 跑各 init」机制：
`config.kv`/`kvlangbrief.kv`/`prompt.kv` 的顶层写语句被 parser 合成为 `local·init` /
`byteseek·init` / `byteseek/prompt·init`，在 layout 期执行一次，把配置、语法速览、系统提示
种进 kvspace（跑完即删 init 子树）。rwfunc（`main`/`mainbrain`/`run`/`llm·call`/`shell·run`/
`python·run`）留在 `/lib` 下持久。之后 `kvlang byteseek·main` 直接驱动 funckey，不再 layout、
不再重跑 init。`byteseek·main` 是 funckey 路径（去 `/lib` 前缀），不是文件路径。

## 状态树布局

```
/byteseek/llm.api            LLM 接口地址（config.kv 种入）
/byteseek/llm.key            LLM 鉴权 key（config.kv 种入，可空）
/byteseek/llm.print          是否打印 LLM 交互（config.kv 种入）
/byteseek/prompt/system      系统提示（prompt.kv 种入）
/lib/byteseek.kvlangbrief    kvlang 语法速览（kvlangbrief.kv 种入；llm·call 拼进 system prompt）
/lib/byteseek/session/<name> llm·call 生成并入库的一次性程序（可寻址/持久）
/vthread/{vid}/‥pc           执行到哪一步（KV 路径字符串，崩溃可恢复）
/lib/<pkg>·<name>/...        编译后函数（签名 + 指令 + 源码 .src）
```

## 提示词与 LLM 参数也是 KV 数据

系统提示不硬编码，而是 `prompt.kv` 的 `lib prompt` 顶层写把提示词落进 `/byteseek/prompt/system`；
语法速览由 `kvlangbrief.kv` 落进 `/lib/byteseek.kvlangbrief`。`llm·call` 每轮把两者拼成
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
包进 `lib byteseek { lib session { lib NAME { … NAME·main() } } }` → `kvlanglayout·vet` 校验 →
通过后 `kvlanglayout·layout` 入库 → 返回入口 `byteseek/session/NAME·init`。生成失败回填
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
`vthread·run` 不同——不新开 vid、不 WATCH 挂起——被调程序里的 rwir（`println`/`shell·run`/…）
由当前驱动就地派发。故一次 REPL 请求、生成程序的执行、其内工具调用全在同一进程同一 vid 内完成，
契合「一个进程 ⟺ 一条 vthread」。

## rwir：全部复用 kvlang 标准集

byteseek 不再注册任何自有 Rust rwir。所需能力全部是 kvlang 标准 rwir（runtime-rs / runtime-c）：

| opcode | 来源 |
|--------|------|
| `print` / `println` / `cerr` / `input` | 标准 term rwir |
| `json·to` / `json·from` | 标准 json rwir |
| `http·call(method,header,url,body) -> resp` | 标准 http rwir |
| `kvlanglayout·vet / ·format / ·layout / ·dump` | 标准 layout rwir |
| `networld/proc·exec(args,envs) -> code, out, err` | 标准 networld rwir（子进程 + 捕获 @ 句柄） |
| `vthread·call(funckey)` | native builtin（同 vid 动态调用） |
| `string·* / kv·* / xv·*` | native builtin |

`shell·run` / `python·run` 是 `lib/` 下的 kv rwfunc：把命令包成 `{"bash","-c",cmd}` /
`{"python3","-c",code}` 交给 `networld/proc·exec`，绑定 stdout/stderr 写槽即捕获（`@[]uint8`
扩展句柄，`println` 读时按 body 前缀 `/networld/{host}/proc` 路由回兑现物理字节），返回 stdout。

## 已验证（0rs）

- `KVLANG_LIB=lib kvlang` 引导：config/语法速览/系统提示三 init 于 layout 期种入，rwfunc 持久。
- `kvlang byteseek·main`：REPL 循环、`exit` 退出、无 key 时 `llm·call` 回填 error 且 `byteseek·run` 跳过。
- `byteseek·run(entry)` → `vthread·call`：同 vid 动态执行入库的 session 程序，其内 rwir 就地派发。
- `shell·run` / `python·run`：经 `networld/proc·exec` 捕获子进程 stdout 并透明兑现。
- `make test`（无网络无 LLM，shm 后端）：vet + session 执行 + shell/python 捕获全链路通过。
