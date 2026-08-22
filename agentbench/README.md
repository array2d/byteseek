# agentbench —— byteseek agent 能力测试基准

对 corebrain（代码脑）做端到端能力测试：给定自然语言任务，agent 生成 kvlang 程序执行，
结果经 `println` 落到终端。每题两个文件：`<name>.question`（任务）与 `<name>.answer`（基准答案）。

## 目录结构

```
agentbench/
  {难度00..10}/{类别}/{name}.question
                        {name}.answer
```

难度在前、类别在后，两级子目录。难度用两位补零 `00`–`10`（共 11 级）。

## 难度量表（0–10）

| 难度 | 说明 |
|------|------|
| 0 | 单步、无工具：直接算术 / 字符串拼接 |
| 1 | 单步单工具（shell/python），或简单循环累加 |
| 2 | 单工具带参数 / 单次算法（数组最值、幂运算） |
| 3 | 单算法 / 字符串统计 / 网络冒烟 |
| 4 | 数组算法 / 回文判断 / dict 成员读 |
| 5 | dict 指针链表 / kvspace 读写 / 中等逻辑 |
| 6 | 多工具组合 / kvspace 多键 / 二分查找 |
| 7 | 抓取→解析→存储 / 嵌套状态树 / 多约束推理 |
| 8 | 自省自改 / 错误恢复 / 嵌套代码生成（需 LLM） |
| 9 | 跨会话记忆 / 开放分解 / 经典谜题 |
| 10 | 开放目标，无固定解（按 rubric 评分） |

## 类别

| 类别 | 内容 |
|------|------|
| arithmetic | 数值计算与算术表达式 |
| string | 字符串拼接 / 反转 / 统计 |
| array | 数组算法（最值 / 求和 / 查找 / 排序） |
| dict | 键值结构与路径指针 |
| tool-shell | shell 工具调用 |
| tool-python | python 工具调用 |
| tool-http | http 网络抓取 |
| kvspace | 状态树读写与持久化 |
| reasoning | 逻辑推理与约束满足 |
| planning | 多步规划与多工具组合 |
| meta | 记忆、自省、自改（高阶） |

## 答案约定

- 确定性题目：`.answer` 即 `println` 的期望输出（裸值，不含换行）。
- 冒烟题目（需真实 LLM / 网络 / 完整 bootstrap）：`.answer` 以 `期望(冒烟):` 开头，给出可验证检查。
- 开放题目（难度 10）：`.answer` 以 `评分标准:` 开头，给出 rubric，无单一标准输出。

## 环境假设

冒烟题依赖完整 byteseek runtime（已 bootstrap，`/lib/byteseek.kvlangbrief` 与
`/byteseek/prompt/system` 已播种）、可用的 `DEEPSEEK_API_KEY`（嵌套代码生成题）与出网能力
（http 题，走 `http_proxy`/`https_proxy` 环境变量）。
