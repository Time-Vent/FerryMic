# FerryMic

桌面端「麦克风 + 音效板」小工具：把你的麦克风和音效混在一起，输出到虚拟麦克风，
游戏语音、会议、直播软件把它当作麦克风，对方就能同时听到你的声音和音效。

A small desktop "mic + soundboard" app: mixes your microphone with sound effects and
sends the result to a virtual microphone, so game voice chat, meetings and streaming
apps hear both your voice and the effects.

## 功能

- ✅ **桌面 App**：独立窗口（Tauri + 系统 WebView），Windows 版约 9 MB，无需浏览器
- ✅ **麦克风 + 音效混合**：自动识别硬件麦克风；音量、静音、电平表；运行中切换设备，断开自动重连
- ✅ **输出音频线路可切换**：设置里选虚拟音频线，不用重启；底部直接提示其他软件里该选哪个麦克风
- ✅ **试听**：音效同时在耳机 / 扬声器里播放，自己也能听到（不含自己的声音，可单独调音量）
- ✅ **音效库 = 磁盘文件夹**：自选位置，子文件夹即分类（可多层）；在资源管理器里放进去的文件刷新后出现
- ✅ **整理音效**：新建 / 重命名 / 删除文件夹，把卡片拖到文件夹即可移动；删除的文件进回收站
- ✅ **卡片快捷控件**：循环 / 单次（双箭头图标）、独占 / 叠加（圆点 / 两个交叠的圆）、倍速都是点一下切换；音量点开竖向滑条
- ✅ **新音效默认设置**：默认独占、单次、0 dB、1.0×，设置里可改，也可一键应用到所有现有音效
- ✅ **进度条拖动**：播放中拖动卡片底部的进度条跳转
- ✅ **全局快捷键**：按键录制设置，游戏全屏时也能触发
- ✅ **外观**：深色 / 浅色 / 跟随系统，主题色 7 种预设 + 自定义；App 图标颜色可跟随主题色
- ✅ **音效资源**：左侧地球按钮打开免费 / 开放音效网站合集（Freesound、Pixabay、Mixkit、BBC 等），可添加自己的网址；
  App 里在预览窗口打开，下载的音频文件自动存进音效库的「下载」文件夹
- ✅ **多语言**：简体中文 / English，设置里切换（默认跟随系统），界面、错误提示、托盘菜单一起切换
- ✅ **托盘**：关闭窗口时询问「最小化到托盘」还是「退出」（可记住选择）
- ✅ **防漏音 / 防反馈**：自动排除会形成反馈环的虚拟线和「立体声混音」这类回录设备
- ✅ **Linux**：自动创建虚拟麦克风（PipeWire / PulseAudio），无需第三方软件
- ✅ **设置内置说明**：基础操作 + 各平台接入游戏的步骤
- ✅ **手机遥控**：左侧一键开关（右上角信号波亮起表示已开启），扫二维码打开；带访问码，即开即关不用重启
- ✅ **浏览器版**：`soundbridge-server` 用浏览器访问，功能与 App 相同

## 快速开始

### Windows 桌面 App（推荐）

```powershell
powershell -ExecutionPolicy Bypass -File .\build-app.ps1
```

产物在 `dist-app\FerryMic\`（`FerryMic.exe` + `WebView2Loader.dll`，两个文件放在一起），
双击 `FerryMic.exe` 即可运行，整个文件夹可以复制到任何位置。

### Linux 桌面 App

```bash
# Debian / Ubuntu 依赖（其它发行版请装对应包）
sudo apt install build-essential pkg-config libasound2-dev libwebkit2gtk-4.1-dev \
     libgtk-3-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pulseaudio-utils
./build-app.sh            # 产物：dist-app/FerryMic/FerryMic
```

> Linux 版目前只做了代码适配，尚未在真实 Linux 系统上实测（开发机为 Windows）。

### 浏览器版

```bash
cd apps/web && pnpm install && pnpm build && cd ../..
cargo run --release --bin soundbridge-server
# 浏览器打开 http://127.0.0.1:8321
```

`start-cable.ps1` / `start-speaker.ps1` 是浏览器版的快捷启动脚本（分别输出到虚拟音频线 / 扬声器）。

## 使用流程

1. **导入音效**：把音频文件拖进窗口（或直接拖到左侧某个文件夹上），或点右上角「导入音效」。
   文件会保存进音效库文件夹。
2. **整理**：左侧「新建文件夹」；文件夹右侧「⋯」可新建子文件夹、重命名、删除；把卡片拖到文件夹上即可移动。
3. **播放**：点卡片播放；播放中拖动进度条跳转，鼠标移到圆形按钮上可停止；右上角「全部停止」。
4. **调整**：卡片底部——
   - 双箭头图标：循环 / 单次（循环时再点一次卡片停止）；
   - 圆点 / 两个交叠的圆：独占 / 叠加（独占会打断其它正在播放的音效）；
   - 音量：点开竖向滑条；倍速：点一下切到下一档，右键回上一档；
   - 「⋯」打开完整编辑（名称、文件夹、快捷键、删除）。
5. **麦克风**：底部麦克风条可静音、调音量、看电平；默认自动选硬件麦克风，也可在设置 →「音频链路」里指定。
6. **接入其他软件**：设置 →「音频链路」→「输出音频线路」选虚拟音频线（如 `CABLE Input`），
   底部会显示「其他软件的麦克风选：CABLE Output」——在要用的软件里把麦克风选成它即可。
   输出到扬声器时底部会变成黄色警告（其他软件收不到）。
7. **试听**：底部耳机按钮开关试听，音效同时在耳机里播放；设备和音量在设置里调。
8. **手机遥控**：点左侧手机图标，开启后用手机扫二维码。
9. **找音效**：点左侧地球图标打开「音效资源」，「预览」在 App 窗口里打开网站试听；
   在那个窗口里点网站的下载按钮，音频文件会直接存进音效库的「下载」文件夹（其它文件照常进系统下载目录）；
   窗口右上角会提示下载进度，已有同名文件时不会重复下载。预览窗口标题栏上的地球按钮（在最小化按钮左边）可在浏览器中打开当前网页。
   网站多了可以用页面右上角的搜索框按名称、网址或介绍筛选。
   列表里的授权说明只是概括，以各网站为准。

**麦克风来源说明**：

- **手机当麦克风**（如 WO Mic 等软件）：电脑端驱动会添加一个录音设备，在设置里选它即可。
- **蓝牙耳机**：Windows 会把耳机切到通话模式（Hands-Free），麦克风为 8/16 kHz 单声道，
  程序会自动换算采样率；此时耳机放音也会变成通话音质，这是蓝牙协议本身的限制。
- 设备中途断开时显示「等待麦克风连接…」，重新连上后自动恢复，无需重启。

**会被拒绝的输入设备**（不限厂商，按设备名判断）：

- 与输出是同一个虚拟设备的两端（如 `CABLE Output` → `CABLE Input`）：会形成反馈环；
- 立体声混音（Stereo Mix）等回录设备：会把游戏等所有正在播放的声音录进去。

蓝牙耳机的麦克风与听筒同名，但不是虚拟设备，不会被误判。

## 平台说明

| 平台 | 虚拟麦克风 | 游戏里选择的麦克风 |
|---|---|---|
| Windows | 安装 VB-Cable 等虚拟音频线；设置里「输出音频线路」选 `CABLE Input` | `CABLE Output` |
| Linux | FerryMic 启动时**自动创建**（需 `pactl`，PipeWire / PulseAudio 均可），退出时移除 | `FerryMic-Microphone` |
| macOS | 安装 BlackHole；设置里「输出音频线路」选 `BlackHole 2ch` | `BlackHole 2ch` |

- Windows：「声音设置」里的默认播放设备保持为耳机或扬声器，不要设成 `CABLE Input`，否则游戏声音也会进麦克风。
- Linux：全局快捷键需要 X11 会话；Wayland 下请改用 X11 会话，或用界面 / 手机遥控触发。
  cpal 在 Linux 走 ALSA，麦克风建议用「自动」，在系统声音设置里选择默认输入设备。
- macOS：尚未实测；首次使用需在「隐私与安全性」中允许麦克风权限。

## 配置

### 音效库

音效库就是一个文件夹（默认 `<数据目录>/sounds`，可在设置里改，可选择「把现有文件一起移过去」）：

- 子文件夹即分类，支持多层；以 `.` 开头的文件和文件夹会被忽略。
- 支持 WAV / MP3 / FLAC / OGG / M4A / AAC。
- 每个音效的设置（循环 / 独占、音量、倍速、快捷键）保存在根目录的 `.soundbridge.json`，
  按相对路径记录，所以整个文件夹可以搬走或同步到别的电脑。
- 在资源管理器里增删改文件后，点左侧「刷新」（或切回窗口时自动扫描）即可同步。
  在资源管理器里改名或移动文件会丢失该音效的设置（App 内操作不会）。
- 大文件夹不会一次性全部读进内存：后台按顺序预加载，累计约 768 MB 后其余音效在第一次播放时再加载。

### 数据目录

| 平台 | 位置 |
|---|---|
| Windows | `%APPDATA%\FerryMic\` |
| Linux | `$XDG_DATA_HOME/FerryMic/`（默认 `~/.local/share/FerryMic/`） |
| macOS | `~/Library/Application Support/FerryMic/` |

`settings.json` 保存麦克风与输出设备、试听、手机遥控（开关与访问码）、外观、语言、关闭窗口行为、音效库位置、新音效默认设置、自己添加的音效网站。旧版的 `library.json`
会在首次启动时自动迁移进音效库文件夹（原文件改名为 `library.v1.json.bak`）。

**改名说明**：本项目原名 SoundBridge。首次启动 FerryMic 时，旧数据目录（如 `%APPDATA%\SoundBridge`）会自动改名为
`%APPDATA%\FerryMic`，设置与音效库原样保留（设置里指向旧目录内部的音效库位置也会跟着更新）。代码内部的 crate 名、
`SOUNDBRIDGE_*` 环境变量和音效库里的 `.soundbridge.json` 保持不变。

### 环境变量（App 与浏览器版通用）

| 变量 | 作用 |
|---|---|
| `SOUNDBRIDGE_OUTPUT_DEVICE` | 输出设备（包含匹配），如 `CABLE Input`，优先于设置里的选择；都没有 = 系统默认（Linux 为自动创建的虚拟麦克风） |
| `SOUNDBRIDGE_INPUT_DEVICE` | 麦克风（包含匹配），优先于上次的选择 |
| `SOUNDBRIDGE_MIC_MUTED` | `1` = 启动时麦克风静音（输出到扬声器时防啸叫） |
| `SOUNDBRIDGE_LAN` | `1` / `0` = 强制开 / 关手机遥控（覆盖设置） |
| `SOUNDBRIDGE_PORT` | 端口，默认 `8321`（App 被占用时自动换端口）；手机遥控用「端口 + 1」 |
| `SOUNDBRIDGE_DATA_DIR` | 数据目录 |

麦克风优先级：`SOUNDBRIDGE_INPUT_DEVICE` > 上次的选择 > 自动（系统默认的硬件麦克风，否则第一个硬件麦克风）。
输出优先级：`SOUNDBRIDGE_OUTPUT_DEVICE` > 上次的选择 > 系统默认。

### 手机遥控

默认**关闭**。点左侧手机图标即可开启，**立即生效**，面板里显示二维码和网址
（手机与电脑需在同一 Wi-Fi；首次开启时系统可能询问防火墙权限）。

- 手机遥控在单独的端口（主端口 + 1）上监听局域网，关闭时这个端口直接停掉；本机界面始终只监听 `127.0.0.1`。
- 网址带访问码 `?t=…`，扫码后记在手机的 Cookie 里；没有访问码的设备打不开。
  「重新生成访问码」后，已扫过码的手机需要重新扫。
- 手机上不能修改设备、设置、音效库位置等电脑端选项，只能播放 / 停止 / 编辑音效。

## 架构

```
apps/app（桌面 App，Tauri）              apps/server（浏览器版入口）
   ├─ 托盘 / 关闭询问 / 图标着色            │
   └─ 原生窗口 ──加载──┐                    │
                       ▼                    ▼
              soundbridge_server（库）：HTTP API + 内嵌前端 apps/web
                ├─ 音效库（library.rs）：磁盘文件夹 + .soundbridge.json
                ├─ 加载线程（loader.rs）：预加载 + 按需加载，唯一触发入口
                ├─ 音效板（soundboard crate）：触发语义 + 混音 + 定位
                ├─ 输出线程（audio.rs）：混音回调 + 运行中切换输出设备
                ├─ 麦克风线程（mic.rs）：采集 + 自动选硬件麦克风 + 断开重连 + 输入检查
                ├─ 试听线程（monitor.rs）：只含音效的混音 → 耳机 / 扬声器（重采样）
                ├─ 手机遥控（remote.rs）：局域网监听（端口 + 1）+ 访问码 + 二维码
                ├─ 全局热键线程（hotkeys.rs）：Win32 消息循环
                ├─ Linux 虚拟麦克风（virtual_mic.rs）：pactl 建 null sink + remap source
                └─ 音频输出（audio-cpal）：声卡 / 虚拟音频线
```

详见 [音效板架构](docs/soundboard-architecture.md)。

## REST API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/health` | 健康检查 |
| GET | `/api/app` | 版本、平台、数据目录、音效库位置、输出设备、外观、手机遥控、窗口选项、虚拟麦克风状态 |
| POST | `/api/settings` | 修改设置 `{theme_mode?, accent?, auto_virtual_mic?, tint_app_icon?, close_action?, language?, sound_defaults?}`（`language`：`system` / `zh-CN` / `en`；`sound_defaults`：`{looping, exclusive, gain_db, speed}`；`custom_sites`：`[{name, url}]`） |
| POST | `/api/window` | 回答关闭询问 `{action: "tray" \| "quit" \| "cancel", remember}`（桌面 App） |
| POST | `/api/open-url` | 打开网址 `{url, title?, external}`：App 里默认在预览窗口打开，`external` 用系统浏览器；只允许 http(s) |
| POST | `/api/pick-folder` | 在电脑上弹出「选择文件夹」对话框，返回 `{path}` |
| GET | `/api/live` | 实时状态：`{playing: [{id, progress}], mic_level, mic_connected, close_prompt}` |
| GET | `/api/devices` | 设备列表：输出带 `kind`（virtual / speaker）与 `game_mic`，输入带 `virtual` |
| POST | `/api/output` | 运行中切换输出音频线路 `{device: "名称" \| null}`，失败恢复原设备 |
| GET / POST | `/api/monitor` | 试听状态 / 设置 `{enabled?, device?, gain_db?}` |
| GET / POST | `/api/remote` | 手机遥控状态 / 开关 `{enabled?, regenerate?}`，即时生效 |
| GET | `/api/remote/qr.svg` | 手机遥控网址的二维码 |
| GET / POST | `/api/mic` | 麦克风状态 / 设置 `{gain_db?, muted?}` |
| POST | `/api/mic/device` | 切换麦克风 `{device: "名称" \| null}`，`null` = 自动（优先硬件麦克风） |
| GET | `/api/library` | 音效库：`{root, folders, sounds}` |
| POST | `/api/library/rescan` | 重新扫描磁盘 |
| POST | `/api/library/location` | 修改音效库位置 `{path, move_contents}` |
| POST | `/api/library/open` | 在系统文件管理器中打开音效库 |
| POST / PATCH / DELETE | `/api/folders` | 新建 `{path}` / 重命名或移动 `{from, to}` / 删除空文件夹 `{path}` |
| GET | `/api/sounds` | 列出音效 |
| POST | `/api/sounds/import` | 上传导入（multipart：`file`，可选 `folder`） |
| PATCH | `/api/sounds/:id` | 编辑 `{name?, folder?, looping?, exclusive?, gain_db?, speed?, hotkey?}`；`name` 改文件名，`folder` 移动文件，`hotkey: null` 清除 |
| DELETE | `/api/sounds/:id` | 删除（文件移到回收站） |
| POST | `/api/sounds/:id/trigger` · `/stop` | 触发 / 停止 |
| POST | `/api/sounds/:id/seek` | 跳转 `{position: 0..1}` |
| POST | `/api/stop-all` | 停止全部 |

## 环境要求（开发）

- **Rust stable**；Windows 用 **MinGW-w64**（`winget install BrechtSanders.WinLibs.POSIX.UCRT`）
- **Node.js LTS + pnpm**（构建前端；`winget install OpenJS.NodeJS.LTS`，`npm install -g pnpm`）
- Windows 10/11 + WebView2 运行时（Win11 自带）；Linux 需 WebKitGTK 4.1、ALSA 开发包、`pactl`
- 虚拟音频线（Windows / macOS）：让游戏把输出当麦克风

> 若源码路径含非 ASCII 字符（如中文目录），GNU 工具链的 `dlltool` 可能失败。
> 建议 `CARGO_TARGET_DIR` 指向纯 ASCII 路径。

**已知问题**：

- 用 GNU 工具链编译 Windows App 时链接器会提示 `.rsrc merge failure: multiple non-default manifests`
  （Tauri 与 Rust 标准库各带一份程序清单，GNU ld 无法合并），目前未发现功能影响。
- 关闭询问、托盘、图标着色只在桌面 App 里有；浏览器版没有这些选项。

## 目录结构

```
soundbridge/
├── crates/
│   ├── audio-types/     # 纯数据类型
│   ├── platform-api/    # 能力 trait 契约
│   ├── dsp/             # 定长处理器
│   ├── audio-graph/     # 顺序处理链
│   ├── audio-source/    # 解码 + 变速源（共享 PCM、定位）
│   ├── audio-engine/    # 三执行器 + DriftController
│   ├── soundboard/      # 音效板核心（多音效 + 触发语义 + 定位）
│   └── application/     # CLI 演示用装配层
├── adapters/
│   └── audio-cpal/      # cpal 实现 AudioBackend
├── apps/
│   ├── app/             # 桌面 App（Tauri 外壳）
│   ├── server/          # 服务库 + 浏览器版入口（音频 + 热键 + 音效库 + API）
│   ├── web/             # 界面（Vite + TS，App / 浏览器 / 手机共用）
│   └── desktop/         # CLI 演示壳
├── build-app.ps1        # Windows 一键构建
├── build-app.sh         # Linux 一键构建
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
