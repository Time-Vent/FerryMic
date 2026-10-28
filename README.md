# SoundBridge

桌面端「麦克风 + 音效板」小工具：把你的麦克风和音效混在一起，输出到虚拟音频线，
游戏语音、会议、直播软件把它当作麦克风，对方就能同时听到你的声音和音效。

A small desktop "mic + soundboard" app: mixes your microphone with sound effects and
sends the result to a virtual audio cable, so game voice chat, meetings and streaming
apps treat it as a microphone.

## 功能

- ✅ **桌面 App**：独立窗口（Tauri + 系统 WebView2），一个 9 MB 左右的 exe，无需浏览器
- ✅ **麦克风 + 音效混合**：麦克风音量、静音、电平表
- ✅ **麦克风随时切换**：支持手机 mic 软件、蓝牙耳机；断开自动重连；可「跟随系统默认」
- ✅ **音效库**：拖入或选择文件导入（WAV/MP3/FLAC/OGG/AAC），自动保存，重启后还在
- ✅ **触发方式**：点按 / 循环 / 独占 / 叠加，每个音效独立设置音量、倍速（变速变调）
- ✅ **全局快捷键**：按键录制设置，游戏全屏时也能触发
- ✅ **防漏音 / 防反馈**：自动排除会形成反馈环的虚拟线和「立体声混音」这类回录设备
- ✅ **手机遥控**（可选）：开启后手机浏览器打开同一套界面，远程触发音效
- ✅ **浏览器版**：`soundbridge-server` 用浏览器访问，功能与 App 相同

## 快速开始

### 桌面 App（推荐）

```powershell
powershell -ExecutionPolicy Bypass -File .\build-app.ps1
```

产物在 `dist-app\SoundBridge\`（`SoundBridge.exe` + `WebView2Loader.dll`，两个文件放在一起），
双击 `SoundBridge.exe` 即可运行，整个文件夹可以复制到任何位置。

### 浏览器版

```bash
cd apps/web && pnpm install && pnpm build && cd ../..
cargo run --release --bin soundbridge-server
# 浏览器打开 http://127.0.0.1:8321
```

`start-cable.ps1` / `start-speaker.ps1` 是浏览器版的快捷启动脚本（分别输出到虚拟音频线 / 扬声器）。

## 使用流程

1. **导入音效**：把音频文件拖进窗口，或点右上角「导入音效」。文件会复制到数据目录，原文件可删。
2. **编辑音效**：鼠标移到卡片上点「⋯」，可改名称、触发方式、音量、倍速、快捷键（点击后直接按组合键）。
3. **播放**：点卡片或按快捷键；播放中的卡片会发光，悬停可停止。右上角「全部停止」。
4. **麦克风**：底部麦克风条可静音、调音量、看电平；左下角「设置」里选麦克风设备。
5. **接入游戏**：输出设为虚拟音频线（见下方环境变量），游戏/会议里把麦克风设为该线的另一端
   （如 `CABLE Output`）。

**麦克风来源说明**：

- **手机当麦克风**（如 WO Mic 等软件）：电脑端驱动会添加一个录音设备，在设置里选它即可。
- **蓝牙耳机**：Windows 会把耳机切到通话模式（Hands-Free），麦克风为 8/16 kHz 单声道，
  程序会自动换算采样率；此时耳机放音也会变成通话音质，这是蓝牙协议本身的限制。
- 设备中途断开时显示「等待麦克风连接…」，重新连上后自动恢复，无需重启。

**会被拒绝的输入设备**（自动从列表隐藏，不限厂商）：

- 与输出是同一个虚拟设备的两端（如 `CABLE Output` → `CABLE Input`）：会形成反馈环；
- 立体声混音（Stereo Mix）等回录设备：会把游戏等所有正在播放的声音录进去。

## 配置

### 数据目录

`%APPDATA%\SoundBridge\`（可用 `SOUNDBRIDGE_DATA_DIR` 指定其它位置）：

| 文件 | 内容 |
|---|---|
| `library.json` | 音效库（名称、文件、触发方式、音量、倍速、快捷键） |
| `sounds/` | 导入时复制进来的音频文件 |
| `settings.json` | 麦克风选择与音量、手机遥控开关 |

### 环境变量（App 与浏览器版通用）

| 变量 | 作用 |
|---|---|
| `SOUNDBRIDGE_OUTPUT_DEVICE` | 输出设备（包含匹配），如 `CABLE Input`；不设 = 系统默认 |
| `SOUNDBRIDGE_INPUT_DEVICE` | 麦克风（包含匹配），优先于上次的选择 |
| `SOUNDBRIDGE_MIC_MUTED` | `1` = 启动时麦克风静音（输出到扬声器时防啸叫） |
| `SOUNDBRIDGE_LAN` | `1` / `0` = 强制开 / 关手机遥控（覆盖设置） |
| `SOUNDBRIDGE_PORT` | 端口，默认 `8321`（App 被占用时自动换端口） |
| `SOUNDBRIDGE_DATA_DIR` | 数据目录 |

麦克风优先级：`SOUNDBRIDGE_INPUT_DEVICE` > 上次的选择 > 系统默认。

### 手机遥控

默认**关闭**，只允许本机访问。在设置里打开后**重启生效**，设置页会显示手机访问地址
（手机与电脑需在同一 Wi-Fi；首次开启时 Windows 可能询问防火墙权限）。
目前没有访问密码，同一局域网内的设备都能打开界面，请只在可信网络开启。

## 架构

```
apps/app（桌面 App，Tauri）              apps/server（浏览器版入口）
   └─ 原生窗口 ──加载──┐                    │
                       ▼                    ▼
              soundbridge_server（库）：HTTP API + 内嵌前端 apps/web
                ├─ 音效库（library.rs）：持久化 + 导入
                ├─ 音效板（soundboard crate）：触发语义 + 混音
                ├─ 麦克风线程（mic.rs）：采集 + 切换 + 断开重连 + 输入检查
                ├─ 全局热键线程（hotkeys.rs）：Win32 消息循环
                └─ 音频输出（audio-cpal）：声卡 / 虚拟音频线
```

```
麦克风（手机 mic / 蓝牙耳机 / USB）────────────┐
                                              ▼
App 窗口 / 浏览器 / 手机 ──HTTP──▶ 音效板 ──混音──▶ 虚拟音频线 ──▶ 游戏 / 会议
                                    ▲
全局热键（游戏里按快捷键）─────────────┘
```

详见 [音效板架构](docs/soundboard-architecture.md)。

## REST API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/health` | 健康检查 |
| GET | `/api/app` | 版本、数据目录、输出设备、端口、手机遥控状态 |
| POST | `/api/settings` | 修改设置 `{lan_remote?}` |
| GET | `/api/live` | 实时状态：正在播放的音效、麦克风电平 |
| GET | `/api/devices` | 输入/输出设备列表（输入已排除反馈环与回录设备） |
| GET | `/api/mic` | 麦克风状态（选择、在用设备、是否连接、音量、静音、电平） |
| POST | `/api/mic` | 设麦克风 `{gain_db?, muted?}`（音量会被记住） |
| POST | `/api/mic/device` | 切换麦克风 `{device: "名称" \| null}`，`null` = 跟随系统默认 |
| GET | `/api/sounds` | 列出音效（含时长、加载错误） |
| POST | `/api/sounds/import` | 上传导入（multipart：`file`，可选 `name`） |
| POST | `/api/sounds` | 按本机路径添加（文件留在原处） |
| PATCH | `/api/sounds/:id` | 编辑 `{name?, trigger_mode?, gain_db?, speed?, hotkey?}`，`hotkey: null` 清除 |
| DELETE | `/api/sounds/:id` | 删除（导入的文件一并删除） |
| POST | `/api/sounds/:id/trigger` · `/stop` | 触发 / 停止 |
| POST | `/api/stop-all` | 停止全部 |

## 环境要求（开发）

- **Rust stable** + **MinGW-w64**（`winget install BrechtSanders.WinLibs.POSIX.UCRT`）
- **Node.js LTS + pnpm**（构建前端；`winget install OpenJS.NodeJS.LTS`，`npm install -g pnpm`）
- Windows 10/11 + WebView2 运行时（Win11 自带）
- 虚拟音频线（如 VB-Cable）：让游戏把输出当麦克风

> 若源码路径含非 ASCII 字符（如中文目录），GNU 工具链的 `dlltool` 可能失败。
> 建议 `CARGO_TARGET_DIR` 指向纯 ASCII 路径。

**已知问题**：用 GNU 工具链编译 App 时链接器会提示
`.rsrc merge failure: multiple non-default manifests`（Tauri 与 Rust 标准库各带一份程序清单，
GNU ld 无法合并），目前未发现功能影响。

## 目录结构

```
soundbridge/
├── crates/
│   ├── audio-types/     # 纯数据类型
│   ├── platform-api/    # 能力 trait 契约
│   ├── dsp/             # 定长处理器
│   ├── audio-graph/     # 顺序处理链
│   ├── audio-source/    # 解码 + 变速源
│   ├── audio-engine/    # 三执行器 + DriftController
│   ├── soundboard/      # 音效板核心（多音效 + 触发语义）
│   └── application/     # CLI 演示用装配层
├── adapters/
│   └── audio-cpal/      # cpal 实现 AudioBackend
├── apps/
│   ├── app/             # 桌面 App（Tauri 外壳）
│   ├── server/          # 服务库 + 浏览器版入口（音频 + 热键 + API）
│   ├── web/             # 界面（Vite + TS，App / 浏览器 / 手机共用）
│   └── desktop/         # CLI 演示壳
├── build-app.ps1        # 一键构建桌面 App
├── docs/                # 架构 + 代码走读 + ADR
└── tests/fixtures/audio/
```

## CLI 工具（辅助）

```bash
soundbridge --list                                    # 列出设备
soundbridge in.wav --render out.wav --speed 2.0        # 离线渲染
soundbridge in.wav --speed 1.5 --file-gain 0           # 实时播放文件+麦克风
```

## 文档

- [音效板架构](docs/soundboard-architecture.md)（主产品）
- [引擎架构](docs/architecture.md)
- [代码走读](docs/code-walkthrough.md)（逐模块解释，学习用）
- [ADR](docs/adr/)

## 许可证

MIT OR Apache-2.0
