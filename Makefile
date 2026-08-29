# byteseek Makefile —— make 编译并安装到 /usr/bin 与 ~/.local/bin（/usr/bin 需 root：sudo -E make）。
# 依赖 .so 由 ci/deps.sh 下载安装到 /usr/lib（kvspace_durable / kvlang_runtime / kvlang_layout）。

BIN := byteseek
PREFIX ?= /usr/bin
CARGO ?= $(shell command -v cargo 2>/dev/null || printf '%s' '$(HOME)/.cargo/bin/cargo')
OWNER ?= $(SUDO_USER)

.PHONY: all install run clean

all: install

install:
	$(CARGO) build --release
	install -d $(HOME)/.local/bin
	install -m 755 target/release/$(BIN) $(HOME)/.local/bin/$(BIN)
	@if [ -n "$(OWNER)" ]; then chown $(OWNER) $(HOME)/.local/bin/$(BIN); fi
	install -d $(PREFIX)
	install -m 755 target/release/$(BIN) $(PREFIX)/$(BIN)
	@echo "✅ 已安装: $(HOME)/.local/bin/$(BIN) 与 $(PREFIX)/$(BIN)"

run: all
	$(HOME)/.local/bin/$(BIN)

clean:
	$(CARGO) clean
