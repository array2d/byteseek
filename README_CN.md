# byteseek

KV 原生的 agent substrate：agent 的"自己"——代码、状态、记忆、执行进度——全部活在
**同一棵可寻址、可持久、可自改的 KV 树**（kvspace，后端 redis/fs/shm/s3）里。LLM、
shell、python、子 agent 通过 kvlang 标准 rwir 与 kv `rwfunc` 成为这棵树里的一等公民。
byteseek 没有自己的宿主语言，也不是可执行文件：它**就是**一棵 `.kv` 代码树，由标准
`kvlang` 工具链驱动。

```bash
KVLANG_LIB=lib kvlang        # 引导：layout lib/ 全部 .kv 进 kvspace，并执行各 init
kvlang byteseek·main         # 运行：驱动已入库的 funckey（进入 REPL）
```

引导执行 layout 期的各 `init`，把配置、kvlang 语法速览、系统提示种进树；运行只驱动持久化的
funckey——无需重编译，因为没有可编译之物。架构与实现细节见 `doc/substrate.md`。

## 发展三阶段（roadmap）

byteseek 的能力沿一条主线演进：agent 逐步把"造工具、管资源、改自己"的权力从人
手里接管进这棵树。

- **corebrain** —— 驱动这棵树的推理核心。
- **rwirext** —— 注册进树的扩展能力（`llm`/`shell`/`python`/`agent` 是最初四个）。
- **extbrain** —— corebrain 在 kvspace 里自主训练/迭代出的、可替换当前 corebrain 的
  下一代推理核心。

**阶段一 —— corebrain 自造代码，人工辅助扩展（当前）。**
corebrain 自行生成 kv 代码并执行；rwirext 扩展库由人工辅助创建，保证高信任度；以
corebrain 的 loop 形式完成任务的迭代执行。`doc/substrate.md` 的"已验证"清单即处于
此阶段。

**阶段二 —— 扩展库成熟，corebrain 自主迭代 extbrain。**
积累出大量成熟的 rwirext 扩展库；corebrain 拥有足够的存算资源，在 kvspace 架构下
自主完成训练与推理，迭代升级自己的 corebrain/extbrain。人从
"辅助造工具"退到"设定目标"，agent 开始自我改进推理核心。

**阶段三 —— AGI。**
按业界通行定义，AGI 是在**认知任务的全部广度**上达到或超过人类水平的系统——而非
某个窄领域：它能泛化到新任务、自主习得新技能、在领域间迁移知识。byteseek 用自己的
架构逼近这一终点，通用智能的每一项特征都对应树里的一个具体机制：

- **开放式技能习得** —— 一项新技能就是写进同一棵树的一段新 rwirext / kv 代码，而非
  重训一个单体大模型。能力增长 = 树的生长。
- **推理核心自改进** —— 经由 extbrain，agent 在架构内训练、验证并替换自己的
  corebrain；改进闭环作用于自身，corebrain / extbrain / rwirext 无固定上限。
- **跨领域迁移** —— 所有技能、记忆与历史共享同一棵可寻址的 kvspace，一个任务里学到的
  知识可被另一个任务直接寻址、复用。
- **可控可监督** —— 整个自改过程始终停留在一棵可寻址、可持久、可自改的 KV 树内（PC 即
  崩溃可恢复的 KV 路径），能力增长因此保持可观测、可审计、可恢复，而非不透明的黑箱。
  人只提供目标、价值与约束。

这是该闭环所指向的终点——一个开放目标，而非已解问题。
