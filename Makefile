# byteseek 全 kvlang（无 Rust，无编译产物）。byteseek 不是可执行文件，而是活在 kvspace 里的
# 一棵 .kv 代码树，由标准 kvlang 工具链（kvlang / kvlanglayout）驱动。
#   deps  装 kvlang 最新 release（bin/kvlang·kvlanglayout·kvspace + 库 + 头 → /usr）
#   boot  layout lib/ 全部 .kv 进 kvspace 并执行各 init（config/语法速览/系统提示种入）
#   run   boot 后进入 REPL（kvlang byteseek·main）
#   test  无网络无 LLM 自检（boot → layout tests/selftest.kv → run selftest·go）
KVSPACE ?= redis://127.0.0.1:6379
export KVSPACE

.PHONY: deps boot run test

deps:
	./ci/deps.sh

boot:
	KVLANG_LIB=lib kvlang

run: boot
	kvlang byteseek·main

test: boot
	kvlanglayout tests/selftest.kv "$(KVSPACE)"
	@out=$$(kvlang selftest·go 2>&1); echo "$$out"; \
	echo "$$out" | grep -q "vet(good)= ok" && \
	echo "$$out" | grep -q "SESSION: selftest-shell" && \
	echo "$$out" | grep -q "PY: 42" && \
	echo "$$out" | grep -q "RUN: ok" && \
	echo "$$out" | grep -q "FAILKIND: fail" && \
	echo "$$out" | grep -q "TRUNC: truncated" && \
	echo "$$out" | grep -q "RUNTIME: runtime" && \
	echo "$$out" | grep -q "TOOLS: true" && \
	echo "$$out" | grep -q "REC: true" && \
	echo "$$out" | grep -q "DENIED: 8" && \
	echo "$$out" | grep -q "ALLOWED: 0" && \
	echo "$$out" | grep -q "ARG: 9" && \
	echo "$$out" | grep -q "TOOLCALL: true" && \
	echo "$$out" | grep -q "MEM: theme" && \
	echo "$$out" | grep -q "EDIT: 1 0 a c" && echo "✅ selftest 通过" || { echo "❌ selftest 失败"; exit 1; }
	@dsn=$${REPL_SMOKE_DSN:-shm:///tmp/byteseek_repl_smoke}; \
	KVLANG_LIB=lib KVSPACE=$$dsn kvlang >/dev/null 2>&1; \
	KVSPACE=$$dsn kvlang -c 'kvspace·del("/byteseek/llm.key") -> _' >/dev/null 2>&1; \
	out=$$(printf 'hi\nexit\n' | KVSPACE=$$dsn timeout 120 kvlang byteseek·main 2>&1); \
	echo "$$out" | grep -q "bye." && ! echo "$$out" | grep -q "TypeError" && echo "✅ repl 冒烟通过" || { echo "❌ repl 冒烟失败"; echo "$$out" | tail -5; exit 1; }
