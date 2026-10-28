# SoundBridge

桌面端实时音频混音与「音效板」工具：管理多个音效、绑定快捷键，随时把音效混入麦克风
输出到声卡或虚拟音频线，使目标应用（游戏语音、会议、直播）把混音当作麦克风输入。

Desktop real-time audio mixing and "soundboard" tool: manage multiple sounds, bind
hotkeys, and mix them into the mic output to a speaker or virtual audio cable, so
target apps (game voice, meetings, streaming) treat the mix as microphone input.

## 核心功能（音效板）

- ✅ **多音效管理**：导入多个 WAV 音效，每个独立配置音量、倍速（变速变调）
- ✅ **触发语义 DIY**：点按 / 循环 / 独占 / 叠加，每种音效可自选
- ✅ **全局快捷键**：系统级热键（游戏全屏也能触发），如 `F1`、`Ctrl+Alt+1`
- ✅ **Web 界面**：浏览器操作，响应式适配桌面和手机
- ✅ **手机遥控**：手机浏览器连电脑，远程触发音效（音频在电脑处理）
- ✅ **输出设备选择**：输出到声卡或虚拟音频线（如 VB-Cable）
- ✅ **离线渲染**：CLI 版支持把处理结果写成 WAV 文件

## 快速开始（音效板）

```bash
# 1. 构建前端（首次需 pnpm install + build）
cd apps/web
pnpm install && pnpm build
cd ../..

# 2. 编译并启动服务
cargo run --bin soundbridge-server

# 3. 浏览器访问
#    桌面：http://127.0.0.1:8321
#    手机（同一局域网）：http://<电脑IP>:8321
```

指定输出设备（声卡 / 虚拟音频线）：

```bash
# PowerShell
$env:SOUNDBRIDGE_OUTPUT_DEVICE = "CABLE Input"   # VB-Cable 等虚拟音频线
cargo run --bin soundbridge-server
```

> 输出设备名用「包含匹配」，例如 `CABLE` 可匹配 `CABLE Input (VB-Audio Virtual Cable)`。
> 可用 `GET /api/devices` 查看完整设备列表。

## 使用流程

1. 浏览器打开界面，在「添加音效」表单填入名称、文件路径（电脑本地 WAV）、触发模式、音量、倍速、快捷键。
2. 点「▶ 播放」或按绑定的快捷键触发音效。
3. 若输出设备选了虚拟音频线，在游戏/会议里把麦克风设为该虚拟音频线，即可「整活」。

## 架构

```
apps/server（主程序 soundbridge-server）
  ├─ Web 服务（axum）：REST API + 静态前端
  ├─ 音效板（soundboard）：多音效 + 触发语义 + 混音
  ├─ 音频输出（audio-cpal）：声卡 / 虚拟音频线
  ├─ 全局热键（global-hotkey）：快捷键触发
  └─ apps/web（Vite + TS）：响应式前端（桌面/手机共用）
```

```
手机 ──浏览器──▶ Web 服务 ──命令──▶ 音效板 ──混音──▶ 声卡/虚拟音频线
                                        ▲
全局热键（游戏里按 F1）──────────────────┘
```

## REST API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/health` | 健康检查 |
| GET | `/api/devices` | 列出输出设备 |
| GET | `/api/sounds` | 列出音效 |
| POST | `/api/sounds` | 添加音效 |
| DELETE | `/api/sounds/:id` | 删除音效 |
| POST | `/api/sounds/:id/trigger` | 触发音效 |
| POST | `/api/sounds/:id/stop` | 停止音效 |
| POST | `/api/sounds/:id/gain` | 设增益 `{gain_db}` |
| POST | `/api/sounds/:id/speed` | 设倍速 `{speed}` |

## CLI 工具（辅助）

```bash
soundbridge --list                                    # 列出设备
soundbridge in.wav --render out.wav --speed 2.0        # 离线渲染
soundbridge in.wav --speed 1.5 --file-gain 0           # 实时播放文件+麦克风
```

## 环境要求

- **Rust stable** + **MinGW-w64**（`winget install BrechtSanders.WinLibs.POSIX.UCRT`）
- **Node.js + pnpm**（构建前端）
- Windows 10/11（其他平台经 cpal 可编译，loopback/虚拟麦克风仅 Windows 规划）

> 若源码路径含非 ASCII 字符（如中文目录），GNU 工具链的 `dlltool` 可能失败。
> 建议 `CARGO_TARGET_DIR` 指向纯 ASCII 路径。

## 目录结构

```
soundbridge/
├── crates/
│   ├── audio-types/     # 纯数据类型
│   ├── platform-api/    # 能力 trait 契约
│   ├── dsp/             # 定长处理器
│   ├── audio-graph/     # 顺序处理链
│   ├── audio-source/    # 变速源（WAV + varispeed）
│   ├── audio-engine/    # 三执行器 + DriftController
│   ├── soundboard/      # 音效板核心（多音效 + 触发语义）
│   └── application/     # App 装配 + Mixer
├── adapters/
│   └── audio-cpal/      # cpal 实现 AudioBackend
├── apps/
│   ├── server/          # 音效板主程序（Web 服务 + 音频 + 热键）
│   ├── web/             # 前端（Vite + TS）
│   └── desktop/         # CLI 演示壳
├── docs/                # 架构 + 代码走读 + ADR
└── tests/fixtures/audio/
```

## 文档

- [架构文档](docs/architecture.md)
- [代码走读](docs/code-walkthrough.md)（逐模块解释，学习用）
- [ADR](docs/adr/)

## 许可证

MIT OR Apache-2.0
