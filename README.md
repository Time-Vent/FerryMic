# SoundBridge

桌面端实时音频混音工具：采集物理麦克风与音频文件，按可调音量与倍速混合，经基础
DSP 处理后输出到声卡（后续阶段支持虚拟音频线，使目标应用把混音当作麦克风输入）。

Desktop real-time audio mixing tool: captures the physical microphone and audio files,
mixes them with adjustable volume and speed, applies basic DSP, and outputs to the
speaker (later phases add virtual audio cable support).

## 当前状态（第一阶段）

已实现「核心引擎 + 桌面壳」：

- ✅ 文件音源（WAV 解码）+ **varispeed 变速**（线性插值重采样，变速变调）
- ✅ 麦克风采集（cpal/WASAPI）
- ✅ 混音（文件 + 麦克风，独立增益，tanh 软限幅）
- ✅ DSP 处理器：Gain（平滑）、Limiter（峰值限幅）、Biquad（RBJ 均衡）
- ✅ 三执行器架构（Control / Decode / Realtime）+ 水位 PI 漂移控制器
- ✅ **离线渲染**（`--render`，不依赖硬件即可听到处理结果）
- ✅ 跨平台后端抽象（cpal：Windows WASAPI / macOS CoreAudio / Linux）

未实现（第二阶段）：系统声音 loopback、虚拟麦克风、插件隔离、协作同步、移动端。

## 环境要求

- **Rust stable**（本项目锁定 `x86_64-pc-windows-gnu` 工具链）
- **MinGW-w64**（GNU 工具链编译需要 `gcc`/`as`/`dlltool`）
  - 可通过 `winget install BrechtSanders.WinLibs.POSIX.UCRT` 安装
- Windows 10/11（其他平台可通过 cpal 编译，但 loopback/虚拟麦克风仅 Windows 规划）

## 构建

```bash
# 编译全部
cargo build

# 运行测试（27 个单元测试）
cargo test

# 编译并运行演示
cargo run --bin soundbridge -- --list
```

> 注意：若源码路径含非 ASCII 字符（如中文目录），GNU 工具链的 `dlltool` 可能失败。
> 建议把 `CARGO_TARGET_DIR` 指向纯 ASCII 路径，例如：
> ```bash
> $env:CARGO_TARGET_DIR = "C:\Users\<你>\AppData\Local\Temp\sb-target"
> ```

## 使用

```bash
# 1. 列出音频设备
soundbridge --list

# 2. 离线渲染：文件 → 变速 → 增益 → 输出 WAV（不依赖硬件）
soundbridge 输入.wav --render 输出.wav --speed 2.0 --file-gain -3

# 3. 实时播放：文件 + 麦克风 → 混音 → 声卡输出
soundbridge 输入.wav --speed 1.5 --file-gain 0 --mic-gain 0
```

### 参数说明

| 参数 | 含义 | 默认 |
|---|---|---|
| `--list` | 列出设备后退出 | — |
| `--render <out.wav>` | 离线渲染到文件 | — |
| `--speed <n>` | 播放倍速（>0；变速变调） | 1.0 |
| `--file-gain <db>` | 文件增益（dB） | 0 |
| `--mic-gain <db>` | 麦克风增益（dB） | 0 |

## 目录结构

```
soundbridge/
├── crates/
│   ├── audio-types/     # 纯数据类型（无依赖）
│   ├── platform-api/    # 能力 trait 契约
│   ├── dsp/             # 定长处理器：gain/limiter/biquad
│   ├── audio-graph/     # 顺序处理链
│   ├── audio-source/    # 变速源：WAV 解码 + varispeed
│   ├── audio-engine/    # 三执行器 + DriftController
│   └── application/     # App 装配 + Mixer
├── adapters/
│   └── audio-cpal/      # cpal 实现 AudioBackend（全平台）
├── apps/
│   └── desktop/         # CLI 演示壳
├── docs/                # 架构文档 + 代码走读 + ADR
└── tests/fixtures/audio/  # 测试音频
```

## 文档

- [架构文档](docs/architecture.md) — 模块边界、线程拓扑、平台能力
- [代码走读](docs/code-walkthrough.md) — 逐模块/结构体/函数解释（学习用）
- [ADR](docs/adr/) — 架构决策记录

## 许可证

MIT OR Apache-2.0
