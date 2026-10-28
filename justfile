# justfile：构建/测试/打包命令封装
# justfile: build/test/package command wrappers
#
# 用法 Usage:
#   just build          编译 workspace
#   just test           运行单元测试
#   just test-rt        运行实时安全测试（占位）
#   just fmt            格式化
#   just lint           clippy 检查（-D warnings）
#   just build-desktop  编译桌面 demo
#   just run-desktop    运行桌面 demo（--list）

set shell := ["powershell", "-NoProfile", "-Command"]

# 默认：编译 + 测试
default: build test

# 编译整个 workspace
build:
    cargo build

# 编译桌面 demo
build-desktop:
    cargo build --bin soundbridge

# 运行单元测试
test:
    cargo test

# 格式化检查
fmt:
    cargo fmt --all -- --check

# clippy（CI 用 -D warnings）
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# 列出音频设备
run-desktop:
    cargo run --bin soundbridge -- --list

# 实时安全测试（第一阶段占位，后续接 tests/rt-safety）
test-rt:
    @echo "tests/rt-safety 尚未实现（第二阶段）"

# 打包移动端（占位，Flutter 不参与 cargo workspace）
package-mobile:
    @echo "apps/mobile 为 Flutter 工程占位，不参与 cargo build"
