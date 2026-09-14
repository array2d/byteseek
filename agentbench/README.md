# agentbench —— HumanEval / SWE-bench 基准入口

agentbench 不再自带题目。它是一个**薄入口**：调用业界两套标准基准（Codex / Claude 评测用的
HumanEval 系列与 SWE-bench 系列），把 byteseek 当作被测 agent 接进去。

```
agentbench/
  bench        入口脚本（bash 壳，exec 到 BENCH_HOME 的 venv）
  bench.py     驱动：题库 → agent → predictions → 官方评测
```

## 快速开始

```bash
agentbench/bench env                                   # 环境自检
agentbench/bench list                                  # suite 与题量
agentbench/bench all humaneval --agent gold            # 验管道（题库自带答案，不调 LLM）
agentbench/bench run humaneval --agent byteseek --limit 10
agentbench/bench eval humaneval
```

## 命令

| 命令 | 作用 |
|------|------|
| `bench env` | 打印框架版本、题库路径与题量、docker、kvlang/kvspace/byteseek 就绪状态 |
| `bench list` | 列出 suite 与题量 |
| `bench run <suite>` | 生成 `predictions.jsonl`（agent 阶段） |
| `bench eval <suite>` | 官方评测（eval 阶段）；默认取该 suite 最近一次 run 的 predictions |
| `bench all <suite>` | run + eval |

`run` 选项：`--agent gold|byteseek`、`--limit N`、`--offset N`、`--task id1,id2`、
`--out DIR`、`--timeout S`（单题 agent 超时）、`--no-boot`。
`eval` 选项：`--predictions FILE`、`--workers N`、`--k 1 10`（human-eval pass@k）、
`--eval-timeout S`、`--dataset-name`（SWE-bench 数据集名）。

退出码：`0` 通过 / `1` 未通过 / `2` 前置条件不满足（缺题库、缺 docker、byteseek 未就绪）/ `3` 参数错误。

## suite 与评测后端

| suite | 题量 | 评测 |
|-------|------|------|
| `humaneval` | 164 | 官方 `human_eval.evaluate_functional_correctness`（pass@k） |
| `humaneval+` | 164 | `evalplus.evaluate`（HumanEvalPlus，base / plus 两档） |
| `swebench-verified` | 500 | `swebench.harness.run_evaluation`（**需 Docker**） |
| `swebench-lite` | 300 | 同上 |

agent 阶段与评测阶段分离，与官方 harness 一致：`run` 只产 predictions，`eval` 才判分。
因此可以本机产 predictions、拿到有 Docker 的机器上判分。

## agent 契约

- **`gold`**：直接用题库自带答案（`canonical_solution` / `patch`），用来验证管道本身，不调 LLM。
- **`byteseek`**：每题生成一个 `.kv`，内容为 `llm·call(<题目提示>) -> e` 加 `byteseek·run(e)`，
  由 `kvlang` 驱动并捕获 stdout。agent 必须把答案包在 `<<<ANSWER>>>` 与 `<<<END>>>` 之间；
  未取到标记时只在日志里提示，prediction 记为空串（不伪造内容）。

  HumanEval 题问「补全该 Python 函数」；SWE-bench 题问「按问题描述产出 unified diff」。
  SWE-bench 默认只取 diff（不检出仓库）；需要 agent 直接改仓库时，在提示里给出检出目录。

## 环境

重依赖装在 `BENCH_HOME`（默认 `/home/peng.li24/benchmarks`，跨 pod 持久），**不进 git 仓库**：

```
$BENCH_HOME/
  venv/                         human-eval + evalplus + swebench + datasets
  data/human-eval/              HumanEval.jsonl.gz（openai/human-eval 官方题）
  data/evalplus/                HumanEvalPlus.jsonl（evalplus release）
  data/swe-bench/               SWE-bench_Verified.jsonl / SWE-bench_Lite.jsonl（HF princeton-nlp）
  runs/<suite>-<agent>-<时间>/  tasks/ logs/ predictions.jsonl run.json
  trash/                        旧 agentbench 题目（已被本入口取代）
```

重装：

```bash
python3 -m venv $BENCH_HOME/venv
$BENCH_HOME/venv/bin/pip install human-eval evalplus swebench datasets huggingface_hub
```

## 已验证（2026-09-13）

| 检查 | 结果 |
|------|------|
| `run humaneval --agent gold` + `eval humaneval` | `pass@1 = 0.9939`（164 题，1 题官方 canonical 解过不了官方测试） |
| `eval humaneval+`（同一批 gold 样本） | `base 0.9878 / plus 0.8537`（canonical 解在 extra tests 上的已知表现） |
| `run swebench-lite --agent gold` | 产出 5 题标准格式 predictions |
| `eval swebench-lite` | 无 docker → 退出码 2，并打印可在有 Docker 机器直接执行的命令 |
| `run humaneval --agent byteseek` | **阻塞**：见下 |

## 已知阻塞：byteseek lib 与 kvlang 版本漂移

`KVLANG_LIB=lib kvlang` 当前有 3 个文件 layout 失败，byteseek 的 agent 链路因此不可用
（`--agent byteseek` 会在引导后立刻报错退出，不再静默产出空答案）：

| 文件 | 报错 | 原因 |
|------|------|------|
| `lib/byteseek/llm.kv` | `member write on undefined container "/tmp/esc"` | `/tmp/esc·sys` 用了 `·`（成员分隔符），新 kvlang 要求先显式声明容器；路径应写 `/tmp/esc/sys` |
| `lib/byteseek/shell.kv` | `container literal {…} on "noenv" needs a map langtype` | 未标注 langtype 的 `{}` 字面量 |
| `lib/byteseek/python.kv` | 同上（`a`） | 同上 |

byteseek `deps.json` 锁的是 kvlang `v0.2.15`，而本机是 `v0.2.18-9`——`/usr/bin/kvlang` 与
`kvlang/bin/kvlang` 两个二进制都同样失败。这与本次改造无关：修 lib 或对齐 deps 后，
`--agent byteseek` 即可跑通。
