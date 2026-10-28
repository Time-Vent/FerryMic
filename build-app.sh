#!/usr/bin/env bash
# 构建 SoundBridge 桌面 App（Linux），输出到 dist-app/SoundBridge/。
# Build the SoundBridge desktop app (Linux) into dist-app/SoundBridge/.
#
# 依赖 Dependencies（Debian / Ubuntu 示例 example）：
#   sudo apt install build-essential pkg-config libasound2-dev libwebkit2gtk-4.1-dev \
#        libgtk-3-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
#        pulseaudio-utils            # 提供 pactl（PipeWire 系统同样需要）/ provides pactl
#   Rust（rustup）、Node.js LTS、pnpm（npm install -g pnpm）
#
# 用法 Usage:  ./build-app.sh
set -euo pipefail
cd "$(dirname "$0")"

# 1. 前端（会被嵌入可执行文件）。Front-end (embedded into the executable).
(cd apps/web && pnpm install --frozen-lockfile && pnpm build)

# 2. App（使用本机默认目标，不受 rust-toolchain.toml 里的 Windows 目标影响）。
#    The app (host target; the Windows target listed in rust-toolchain.toml is unused).
cargo build --release -p soundbridge-app

# 3. 打包目录。Package folder.
target_dir="${CARGO_TARGET_DIR:-target}"
out="dist-app/SoundBridge"
mkdir -p "$out"
cp "$target_dir/release/SoundBridge" "$out/"
ls -lh "$out"
echo "OK -> $out/SoundBridge"
