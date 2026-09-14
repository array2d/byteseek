#!/usr/bin/env bash
# byteseek 的唯一依赖 = kvlang。不再有 deps.json：kvlang 的 release 是自包含的——
# bin/{kvlang,kvlanglayout,kvspace} + lib/ + lib/kvspace/（两个后端）+ include/，
# 一次装齐 /usr，本地与 CI 共用。
#
#   KVLANG_VERSION=<tag>   钉版本（如 v0.2.18）；默认取**最新** release。
#
# 版本号解析走带 token 的 GitHub API（GITHUB_TOKEN / GH_TOKEN / `gh auth token`）——
# 匿名 api.github.com 常被限流成 403，而 release 包本身是公开的、下载不需要 token。
set -euo pipefail

URL=${KVLANG_INSTALL_URL:-https://raw.githubusercontent.com/array2d/kvlang/master/install.sh}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$URL" -o "$tmp/install.sh"

tag="${KVLANG_VERSION:-}"
if [ -z "$tag" ]; then
  token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
  if [ -z "$token" ] && command -v gh >/dev/null 2>&1; then
    token=$(gh auth token 2>/dev/null || true)
  fi
  if [ -n "$token" ]; then
    tag=$(curl -fsSL -H "Authorization: Bearer $token" \
          https://api.github.com/repos/array2d/kvlang/releases/latest |
          sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
  fi
fi

# 平台识别 + sha256 校验 + 安装都在 install.sh 里；tag 为空时它自己取最新
VERSION="$tag" sh "$tmp/install.sh"

# install.sh 把库放 <prefix>/lib/kvspace，但 kvspace CLI 以 libkvspace.so.1 直接链接该目录，
# 而它不在默认加载路径里（kvlang/kvlanglayout 自带 rpath，kvspace 没有）→ 补 loader 路径。
if [ "$(uname -s)" = Linux ]; then
  prefix="${PREFIX:-/usr}"
  echo "$prefix/lib/kvspace" | sudo tee /etc/ld.so.conf.d/kvspace.conf >/dev/null
  sudo ldconfig
fi
