# byteseek Makefile —— 依赖 .so 由 ci/deps.sh 下载到 libso/，make 只编译自身。
#   libso/lib/libkvspace_durable.so  (byteseek 自持 kvspace 句柄 + TLV)
#   libso/lib/libkvlang_runtime.so   (模式2 执行 + rwirext 宿主)
#   libso/lib/libkvlang_layout.so    (.kv 编译入库)

BIN := byteseek

.PHONY: all run clean

all:
	cargo build --release
	cp target/release/$(BIN) ./$(BIN)
	@echo "✅ 已安装: $(CURDIR)/$(BIN)"

run: all
	./$(BIN)

clean:
	cargo clean
	rm -f ./$(BIN)
