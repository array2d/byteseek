#!/usr/bin/env python3
"""agentbench —— HumanEval / SWE-bench 基准入口。

  bench env                              环境自检（框架 / 题库 / byteseek / docker）
  bench list                             列出 suite 与题量
  bench run <suite> [选项]               生成 predictions（agent 阶段）
  bench eval <suite> [选项]              官方评测（eval 阶段）
  bench all <suite> [选项]               run + eval

suite:  humaneval | humaneval+ | swebench-verified | swebench-lite
agent:  gold（题库自带答案，验管道） | byteseek（驱动 byteseek 代码脑）

退出码：0 通过 / 1 未通过 / 2 前置条件不满足（缺题库、缺 docker、byteseek 不可用）/ 3 参数错误。
"""

import argparse
import gzip
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

BENCH_HOME = Path(os.environ.get("BENCH_HOME", "/home/peng.li24/benchmarks"))
DATA = BENCH_HOME / "data"
RUNS = BENCH_HOME / "runs"
REPO = Path(__file__).resolve().parent.parent
KVLANG = os.environ.get("KVLANG", "kvlang")
KVSPACE = os.environ.get("KVSPACE", "redis://127.0.0.1:6379")

BEGIN = "<<<ANSWER>>>"
END = "<<<END>>>"

SUITES = {
    "humaneval": {"kind": "humaneval", "file": DATA / "human-eval/HumanEval.jsonl.gz"},
    "humaneval+": {"kind": "humaneval", "file": DATA / "evalplus/HumanEvalPlus.jsonl"},
    "swebench-verified": {"kind": "swebench", "file": DATA / "swe-bench/SWE-bench_Verified.jsonl"},
    "swebench-lite": {"kind": "swebench", "file": DATA / "swe-bench/SWE-bench_Lite.jsonl"},
}


# ── 题库 ────────────────────────────────────────────────────────────────
def load_tasks(suite):
    spec = SUITES[suite]
    path = spec["file"]
    if not path.exists():
        die(2, f"题库缺失：{path}（先跑 bench env 看安装状态）")
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt", encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def task_id_of(task):
    return task.get("task_id") or task["instance_id"]


# ── agent 阶段 ──────────────────────────────────────────────────────────
def kv_literal(text):
    """Python 串 → kvlang 双引号字面量（kvlang 与主流语言一致的 \\n \\" \\\\ 转义）。"""
    return json.dumps(text, ensure_ascii=False)


def boot_byteseek():
    env = dict(os.environ, KVLANG_LIB="lib", KVSPACE=KVSPACE)
    r = subprocess.run([KVLANG], cwd=REPO, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        die(2, f"byteseek 引导失败：\n{r.stdout}{r.stderr}")
    if not kvspace_get("/lib/llm·call/[0,0]"):
        die(2, "byteseek 未就绪：/lib/llm·call 缺失——lib/llm.kv 与当前 kvlang 版本不兼容"
               "（`KVLANG_LIB=lib kvlang` 的完整报错见上）。修好 lib 再跑 --agent byteseek。")
    return r.stdout + r.stderr


def kvspace_get(key):
    r = subprocess.run(["kvspace", "get", key], capture_output=True, text=True,
                       env=dict(os.environ, KVSPACE=KVSPACE))
    return "" if "(nil)" in r.stdout or r.returncode else r.stdout.strip()


def prompt_for(kind, task, repo_dir=None):
    if kind == "humaneval":
        return (
            "用 byteseek 完成下面的 Python 编程题：补全函数，把**完整可运行的 Python 实现**"
            "打印出来（不要解释、不要额外文字）。\n"
            f"输出必须严格包在 {BEGIN} 与 {END} 之间。\n\n"
            f"题目：\n{task['prompt']}\n"
        )
    where = f"代码仓库已检出在 {repo_dir}，可用 shell 工具查阅与修改。\n" if repo_dir else ""
    return (
        "用 byteseek 修复下面的软件缺陷，输出一个可 patch 的 unified diff"
        "（`git diff` 格式，不要解释）。\n"
        f"输出必须严格包在 {BEGIN} 与 {END} 之间。\n{where}\n"
        f"仓库：{task['repo']}\n基线 commit：{task['base_commit']}\n"
        f"问题描述：\n{task['problem_statement']}\n"
    )


def extract(text):
    # 提示词与生成源码里都会回显标记（llm.print=1 会打印需求与生成代码），
    # 所以取**最后**一段：那才是 agent 真正打印的答案。
    ms = re.findall(re.escape(BEGIN) + r"\s*(.*?)\s*" + re.escape(END), text, re.S)
    return ms[-1] if ms else ""


def strip_prompt(block, prompt):
    """agent 若把原题函数签名一起打印了，剥掉前缀，只留 completion。"""
    head = prompt.strip().splitlines()[0].strip() if prompt.strip() else ""
    pos = block.find(head) if head else -1
    return block[pos + len(head):] if pos >= 0 else block


def run_byteseek(kind, task, outdir, timeout):
    tid = task_id_of(task).replace("/", "_")
    tasks_dir = outdir / "tasks"
    tasks_dir.mkdir(parents=True, exist_ok=True)
    kvfile = tasks_dir / f"{tid}.kv"
    kvfile.write_text(
        "rwfunc test() -> () {\n"
        f"\tllm·call({kv_literal(prompt_for(kind, task))}) -> e\n"
        "\tbyteseek·run(e)\n"
        "}\n",
        encoding="utf-8",
    )
    env = dict(os.environ, KVSPACE=KVSPACE)
    try:
        r = subprocess.run([KVLANG, str(kvfile)], cwd=REPO, env=env,
                           capture_output=True, text=True, timeout=timeout)
        raw = r.stdout + r.stderr
    except subprocess.TimeoutExpired as e:
        raw = (e.stdout or "") + (e.stderr or "") + f"\n[bench] 超时 {timeout}s"
    (outdir / "logs").mkdir(parents=True, exist_ok=True)
    (outdir / "logs" / f"{tid}.log").write_text(raw, encoding="utf-8")
    return extract(raw), raw


def predict_gold(kind, task):
    if kind == "humaneval":
        sol = task["canonical_solution"]
        return sol if task.get("prompt", "") in sol else task["prompt"] + sol
    return task["patch"]


def cmd_run(a):
    kind = SUITES[a.suite]["kind"]
    tasks = load_tasks(a.suite)
    if a.task:
        want = set(a.task.split(","))
        tasks = [t for t in tasks if task_id_of(t) in want]
    tasks = tasks[a.offset:]
    if a.limit:
        tasks = tasks[: a.limit]
    if not tasks:
        die(3, "没有选中任何题目")

    outdir = Path(a.out) if a.out else RUNS / f"{a.suite}-{a.agent}-{time.strftime('%Y%m%d-%H%M%S')}"
    outdir.mkdir(parents=True, exist_ok=True)
    if a.agent == "byteseek" and not a.no_boot:
        print("[bench] 引导 byteseek ...", file=sys.stderr)
        boot_byteseek()

    preds = []
    for i, task in enumerate(tasks, 1):
        tid = task_id_of(task)
        if a.agent == "gold":
            ans = predict_gold(kind, task)
        else:
            ans, raw = run_byteseek(kind, task, outdir, a.timeout)
            if not ans:
                print(f"[bench] {i}/{len(tasks)} {tid}: 未取到答案标记，原始输出见 logs/", file=sys.stderr)
        if kind == "humaneval":
            completion = strip_prompt(ans, task["prompt"])
            preds.append({"task_id": tid, "completion": completion, "solution": task["prompt"] + completion})
        else:
            preds.append({"instance_id": tid, "model_patch": ans, "model_name_or_path": f"byteseek-{a.agent}"})
        print(f"[bench] {i}/{len(tasks)} {tid}: {len(ans)} 字符", file=sys.stderr)

    pred_file = outdir / "predictions.jsonl"
    with pred_file.open("w", encoding="utf-8") as fh:
        for p in preds:
            fh.write(json.dumps(p, ensure_ascii=False) + "\n")
    (outdir / "run.json").write_text(
        json.dumps({"suite": a.suite, "agent": a.agent, "n": len(preds),
                    "predictions": str(pred_file)}, ensure_ascii=False, indent=1))
    print(pred_file)
    return 0


# ── eval 阶段 ───────────────────────────────────────────────────────────
def cmd_eval(a):
    kind = SUITES[a.suite]["kind"]
    pred = Path(a.predictions) if a.predictions else latest_predictions(a.suite)
    if not pred.exists():
        die(2, f"predictions 不存在：{pred}（先 bench run）")
    if kind == "humaneval":
        return eval_humaneval(a.suite, pred, a)
    return eval_swebench(a.suite, pred, a)


def latest_predictions(suite):
    cands = sorted(RUNS.glob(f"{suite}-*/predictions.jsonl"), key=lambda p: p.stat().st_mtime)
    return cands[-1] if cands else RUNS / f"{suite}-none/predictions.jsonl"


def eval_humaneval(suite, pred, a):
    if suite == "humaneval":
        from human_eval.evaluation import evaluate_functional_correctness
        res = evaluate_functional_correctness(
            str(pred), problem_file=problems_for(suite, pred), k=a.k,
            n_workers=a.workers, timeout=a.eval_timeout)
        print(json.dumps(res, ensure_ascii=False))
        return 0 if res.get(f"pass@{a.k[0]}", 0) > 0 else 1
    need = {t["task_id"] for t in load_tasks(suite)}
    have = {json.loads(l)["task_id"] for l in pred.open(encoding="utf-8") if l.strip()}
    if have != need:
        die(2, f"evalplus 要求样本覆盖全部 {len(need)} 题，当前只有 {len(have & need)} 题"
               f"（先 bench run {suite} --agent ... 跑全量）")
    r = subprocess.run([sys.executable, "-m", "evalplus.evaluate", "--samples", str(pred),
                        "--dataset", "humaneval", "--parallel", str(a.workers)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stdout + r.stderr)
        return r.returncode
    res = Path(str(pred).replace(".jsonl", "_eval_results.json"))
    if res.exists():
        ev = json.loads(res.read_text())["eval"]
        n = len(ev)
        base = sum(1 for v in ev.values() if v[0]["base_status"] == "pass")
        plus = sum(1 for v in ev.values() if v[0]["plus_status"] == "pass")
        print(json.dumps({"n": n, "base_pass@1": round(base / n, 4), "plus_pass@1": round(plus / n, 4),
                          "results": str(res)}, ensure_ascii=False))
        return 0 if plus > 0 else 1
    print(r.stdout + r.stderr)
    return r.returncode


def problems_for(suite, pred):
    """human-eval 要求样本覆盖题库全部题目；跑子集时裁一份同名题库陪跑。"""
    src = SUITES[suite]["file"]
    ids = {json.loads(l)["task_id"] for l in pred.open(encoding="utf-8") if l.strip()}
    rows = load_tasks(suite)
    if {r["task_id"] for r in rows} == ids:
        return str(src)
    out = pred.parent / "problems.jsonl"
    with out.open("w", encoding="utf-8") as fh:
        for r in rows:
            if r["task_id"] in ids:
                fh.write(json.dumps(r, ensure_ascii=False) + "\n")
    return str(out)


def eval_swebench(suite, pred, a):
    if not shutil.which("docker"):
        print(
            "SWE-bench 官方评测需要 Docker（每个实例在自己的容器里装环境、跑测试）。\n"
            f"当前环境没有 docker。predictions 已就绪，可在有 Docker 的机器上执行：\n\n"
            f"  python -m swebench.harness.run_evaluation \\\n"
            f"    --dataset_name {a.dataset_name or 'princeton-nlp/SWE-bench_Lite'} \\\n"
            f"    --predictions_path {pred} --max_workers {a.workers} \\\n"
            f"    --run_id bench-{time.strftime('%Y%m%d-%H%M%S')} --cache_level env\n\n"
            "或只做「能跑通」的轻量验证：byteseek 生成的 patch 非空即视为产出成功。",
            file=sys.stderr)
        return 2
    from swebench.harness.run_evaluation import main as run_evaluation
    run_id = f"bench-{time.strftime('%Y%m%d-%H%M%S')}"
    sys.argv = ["run_evaluation", "--dataset_name", a.dataset_name or "princeton-nlp/SWE-bench_Lite",
                "--predictions_path", str(pred), "--max_workers", str(a.workers),
                "--run_id", run_id, "--cache_level", "env"]
    run_evaluation()
    return 0


# ── 自检 ────────────────────────────────────────────────────────────────
def cmd_env(_a):
    def ver(mod):
        try:
            __import__(mod)
        except Exception as e:
            return f"缺失({e.__class__.__name__})"
        try:
            from importlib.metadata import version
            return version(mod)
        except Exception:
            return "已装(版本未知)"

    print(f"BENCH_HOME : {BENCH_HOME}")
    print(f"python     : {sys.version.split()[0]} ({sys.executable})")
    print(f"frameworks : human-eval {ver('human_eval')} | evalplus {ver('evalplus')} | "
          f"swebench {ver('swebench')} | datasets {ver('datasets')}")
    print(f"docker     : {shutil.which('docker') or '缺失（SWE-bench 官方评测不可用）'}")
    print(f"kvlang     : {shutil.which(KVLANG) or '缺失'}   KVSPACE={KVSPACE}")
    print(f"byteseek   : {REPO}")
    for suite in SUITES:
        path = SUITES[suite]["file"]
        n = len(load_tasks(suite)) if path.exists() else 0
        print(f"  {suite:<18} {n:>4} 题  {path if path.exists() else '缺失'}")
    for key in ("/byteseek/llm.key", "/byteseek/prompt/system"):
        r = subprocess.run(["kvspace", "get", key], capture_output=True, text=True,
                           env=dict(os.environ, KVSPACE=KVSPACE))
        ok = "(nil)" not in r.stdout
        print(f"kvspace {key:<22} {'已种入' if ok else '未种入（先 KVLANG_LIB=lib kvlang）'}")
    return 0


def cmd_list(_a):
    for suite, spec in SUITES.items():
        n = len(load_tasks(suite)) if spec["file"].exists() else 0
        print(f"{suite:<18} {spec['kind']:<10} {n:>4} 题")
    return 0


def die(code, msg):
    print(f"[bench] {msg}", file=sys.stderr)
    sys.exit(code)


def main():
    ap = argparse.ArgumentParser(prog="bench", description="HumanEval / SWE-bench 基准入口")
    sub = ap.add_subparsers(dest="cmd", required=True)

    sub.add_parser("env", help="环境自检").set_defaults(fn=cmd_env)
    sub.add_parser("list", help="列出 suite 与题量").set_defaults(fn=cmd_list)

    def add_common(p):
        p.add_argument("suite", choices=list(SUITES))
        p.add_argument("--predictions", help="predictions.jsonl（默认取最近一次 run）")
        p.add_argument("--workers", type=int, default=8)
        p.add_argument("--timeout", type=int, default=300)

    r = sub.add_parser("run", help="生成 predictions")
    add_common(r)
    r.add_argument("--agent", choices=["gold", "byteseek"], default="byteseek")
    r.add_argument("--limit", type=int, default=0, help="只跑前 N 题（0 = 全部）")
    r.add_argument("--offset", type=int, default=0)
    r.add_argument("--task", help="逗号分隔的 task_id / instance_id 白名单")
    r.add_argument("--out", help="输出目录（默认 $BENCH_HOME/runs/<suite>-<agent>-<时间>）")
    r.add_argument("--no-boot", action="store_true", help="跳过 byteseek 引导")
    r.set_defaults(fn=cmd_run)

    def add_eval_args(p):
        p.add_argument("--k", type=int, nargs="+", default=[1], help="pass@k（humaneval）")
        p.add_argument("--eval-timeout", type=float, default=10.0, help="单题执行超时秒数")
        p.add_argument("--dataset-name", help="SWE-bench 数据集名（默认 SWE-bench_Lite）")

    e = sub.add_parser("eval", help="官方评测")
    add_common(e)
    add_eval_args(e)
    e.set_defaults(fn=cmd_eval)

    x = sub.add_parser("all", help="run + eval")
    add_common(x)
    add_eval_args(x)
    x.add_argument("--agent", choices=["gold", "byteseek"], default="byteseek")
    x.add_argument("--limit", type=int, default=0)
    x.add_argument("--task")
    x.set_defaults(fn=None)

    a = ap.parse_args()
    if a.cmd == "all":
        rc = cmd_run(argparse.Namespace(**{**vars(a), "offset": 0, "out": None, "no_boot": False}))
        if rc:
            return rc
        return cmd_eval(a)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
