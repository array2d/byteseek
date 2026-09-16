#!/usr/bin/env bash
# 停点规划回归：模型是否**自己**判断该不该停（需要真实 LLM）。
#   用例1 确定性任务（程序自己能判断）→ 期望 0 次停点
#   用例2 需要语义判断的任务（写代码时看不到内容）→ 期望 ≥1 次停点
# 用法：tests/stopplan.sh [任务名...]（默认两个用例都跑）
set -uo pipefail
cd "$(dirname "$0")/.."
export KVSPACE=${KVSPACE:-redis://127.0.0.1:6379}
export KVLANG_LIB=lib

D=/tmp/stopplan
mkdir -p "$D"
printf 'x\nb\ny\n' > "$D/b.txt"
printf 'def f(x):\n    return x + 1\n' > "$D/a.txt"

KVLANG_LIB=lib kvlang >/dev/null 2>&1 || { echo "byteseek 引导失败"; exit 2; }

run_case() { # $1=用例名 $2=期望(0|1) $3=任务
  local name=$1 want=$2 task=$3 log="/tmp/stopplan-$1.log"
  printf '%s\n' "$task" | timeout 600 kvlang byteseek·main >"$log" 2>&1
  local got
  got=$(grep -c "任务执行到停点" "$log")
  local ok="FAIL"
  if [ "$want" = "0" ] && [ "$got" = "0" ]; then ok="PASS"; fi
  if [ "$want" = "1" ] && [ "$got" -ge 1 ]; then ok="PASS"; fi
  printf '%s  %-14s 停点=%s（期望%s）  log=%s\n' "$ok" "$name" "$got" "$want" "$log"
}

if [ $# -gt 0 ]; then
  for n in "$@"; do
    case "$n" in
      deterministic) run_case deterministic 0 "读 $D/b.txt，如果含 b 就替换成 c" ;;
      judgement) run_case judgement 1 "读 $D/a.txt，判断它属于哪类内容（代码/散文/数据），把结论写进 $D/kind" ;;
      *) echo "未知用例：$n" ;;
    esac
  done
else
  run_case deterministic 0 "读 $D/b.txt，如果含 b 就替换成 c"
  run_case judgement 1 "读 $D/a.txt，判断它属于哪类内容（代码/散文/数据），把结论写进 $D/kind"
fi
