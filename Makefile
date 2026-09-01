# byteseek 全 kvlang（无 Rust，无编译产物）。byteseek 不是可执行文件，而是活在 kvspace 里的
# 一棵 .kv 代码树，由标准 kvlang 工具链（kvlang / kvlanglayout）驱动。
#   deps  下载 ABI .so 到 /usr/lib（kvlang 二进制需另装，见 README）
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
	echo "$$out" | grep -q "PY: 42" && echo "✅ selftest 通过" || { echo "❌ selftest 失败"; exit 1; }
