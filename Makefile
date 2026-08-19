# byteseek Makefile —— `make` 一键编译并安装到仓库根目录。
#
# lib/   —— byteseek 自带的 .kv 代码（随仓库入库，启动时 layout 进 kvspace）。
# libso/ —— 三方 durable(redis) .so（被 gitignore，此处从相邻仓库构建产物同步）：
#   libkvspace_durable.so  ← ../kvspace-durable   (cargo, byteseek 自持 kvspace 句柄 + TLV)
#   libkvlang_runtime.so   ← ../kvlang            (make durable, C runtime + rwirext 宿主)
#   libkvlang_layout.so    ← ../kvlang            (make durable, Rust cdylib, .kv 编译入库)
# runtime 与 layout 由 kvlang 的 `make durable` 一并产出（grouped target）。

BIN     := byteseek
LIB_DIR := libso
WS      := ..
KVLANG  := $(WS)/kvlang
KVSPACE := $(WS)/kvspace-durable

SO_KVSPACE := $(KVSPACE)/target/release/libkvspace_durable.so
SO_RUNTIME := $(KVLANG)/bin/libkvlang_runtime.so
SO_LAYOUT  := $(KVLANG)/layout/target/release/libkvlang_layout.so

.PHONY: all libs run clean

# 默认目标：同步 .so → cargo 编译 → 安装二进制到仓库根目录。
all: libs
	cargo build --release
	cp target/release/$(BIN) ./$(BIN)
	@echo "✅ 已安装: $(CURDIR)/$(BIN)"

# 同步三方 durable .so 进 lib/（缺失则先构建相邻仓库）。
libs: $(LIB_DIR)/libkvspace_durable.so $(LIB_DIR)/libkvlang_runtime.so $(LIB_DIR)/libkvlang_layout.so

$(LIB_DIR)/libkvspace_durable.so: $(SO_KVSPACE) | $(LIB_DIR)
	cp $< $@
$(LIB_DIR)/libkvlang_runtime.so: $(SO_RUNTIME) | $(LIB_DIR)
	cp $< $@
$(LIB_DIR)/libkvlang_layout.so: $(SO_LAYOUT) | $(LIB_DIR)
	cp $< $@

$(LIB_DIR):
	mkdir -p $@

$(SO_KVSPACE):
	cargo build --release --manifest-path $(KVSPACE)/Cargo.toml

$(SO_RUNTIME) $(SO_LAYOUT) &:
	$(MAKE) -C $(KVLANG) durable

run: all
	./$(BIN)

clean:
	cargo clean
	rm -f ./$(BIN) $(LIB_DIR)/*.so
