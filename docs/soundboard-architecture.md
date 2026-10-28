# 音效板架构（Soundboard）

> 本文档描述「音效板」功能的技术架构：麦克风 + 音效混合、音效库、快捷键触发、桌面 App、
> 界面、手机遥控。

## 1. 定位

音效板是 SoundBridge 的**核心产品形态**：采集真实麦克风，管理多个音效并绑定快捷键，
把「你的声音 + 音效」混在一起输出到声卡 / 虚拟音频线。目标应用（游戏语音、会议、直播）
把虚拟音频线当麦克风，对方就能同时听到人声和音效。

## 2. 模块架构

两个宿主共用同一个服务库，界面也是同一套：

```
apps/app（桌面 App，Tauri）        apps/server/src/main.rs（浏览器版 soundbridge-server）
  └─ 进程内启动服务 + 原生窗口           └─ 启动服务，用浏览器访问
                 \                         /
                  apps/server（库 soundbridge_server）
                    ├─ lib.rs        启动流程 run()、监听地址、内嵌前端（rust-embed）
                    ├─ api.rs        HTTP API 处理函数 + AppState
                    ├─ library.rs    音效库持久化（library.json）+ 导入文件管理（sounds/）
                    ├─ audio.rs      输出流：SoundBoard 混音 + 叠加麦克风 + 限幅；PlayingSet
                    ├─ mic.rs        麦克风线程：打开/切换/断开重连/跟随默认、输入设备检查
                    ├─ settings.rs   设置持久化（麦克风、手机遥控）+ 数据目录
                    ├─ hotkeys.rs    全局热键线程（Win32 消息循环）
                    └─ （依赖）soundboard crate → audio-source（解码 + 共享 PCM + 变速）
apps/web（Vite + TS，无框架）  界面，构建产物嵌入服务库
```

```
crates/soundboard/
  ├─ TriggerMode    触发模式：OneShot / Loop / Exclusive / Overlap
  ├─ SoundSpec      音效定义：名称/文件/模式/增益/变速/快捷键
  ├─ LoadedSound    已解码音效 = SoundSpec + 共享 PCM（控制侧解码）
  ├─ BoardCommand   控制命令：AddSound(LoadedSound) / Trigger / Stop / SetGain / SetSpeed / SetMode ...
  ├─ Instance       播放实例（克隆的 VarispeedSource + 增益/倍速/采样率换算比）
  └─ SoundBoard     音效板：管理定义与实例，fill 混音（实时线程独占）
```

## 3. 线程拓扑

| 线程 | 职责 | 实时约束 |
|---|---|---|
| tokio 线程池 | axum 服务器，REST API；解码、设备枚举、等待回复都在 `spawn_blocking` 中 | 无 |
| 麦克风线程 `soundbridge-mic` | 持有输入流；处理切换请求；每秒巡检断开/默认设备变化 | 无 |
| 热键线程 `soundbridge-hotkeys` | 创建热键管理器 + Win32 消息循环；处理绑定请求，按下发 Trigger | 无 |
| cpal 输入回调线程 | 麦克风 → 下混单声道 → 重采样到输出采样率 → 写 SPSC 环 | **实时** |
| cpal 输出回调线程 | 消费命令(≤32) → SoundBoard::fill → 发布播放状态 → 叠加麦克风 → 限幅 → 写输出 | **实时** |
| （App）Tauri 主线程 | 原生窗口与 WebView2；服务在单独线程的 tokio 运行时里 | 无 |

**数据流**：

```
麦克风(手机 mic / 蓝牙耳机 / USB) ─输入回调─▶ 单声道 + 重采样 ─SPSC 环─┐
                                                                     ▼
Web API / 全局热键 ──BoardCommand──▶ 输出回调：SoundBoard 混音 + 麦克风 ─▶ 声卡 / 虚拟音频线
```

**实时安全**：输出/输入回调不分配、不加锁、不做 IO。音频文件在「添加音效」时由控制侧解码
（`LoadedSound::load`），触发时实时线程只克隆 `Arc` 共享的 PCM。

## 4. 触发语义

| 模式 | 行为 |
|---|---|
| `OneShot`（点按） | 每按一次从头播一遍，播完自动停止 |
| `Loop`（循环） | 按一下开始循环，再按停止 |
| `Exclusive`（独占） | 播放时停止其它所有音效 |
| `Overlap`（叠加） | 可与其它音效同时播放 |

每个音效独立配置：触发模式 + 增益（dB）+ 倍速（varispeed，变速变调）。
同时播放的实例上限 `MAX_INSTANCES = 32`（预分配），满了替换最早的实例。

## 5. 混音（SoundBoard::fill）

- 遍历所有播放实例，每个实例 `VarispeedSource::fill` 到 scratch，按增益累加到输出。
- **采样率换算**：步进 = 用户倍速 × 源采样率 / 输出采样率，44.1 kHz 文件在 48 kHz 输出上不变调。
- **分块**：驱动单次要的帧数超过 scratch 容量时分块处理，不会越界。
- 实例源声道转换到输出声道（mono 复制 / stereo 直接 / 其它取交集）。
- 非循环实例播放完（exhausted）自动移除；循环实例 reset 继续。

## 6. 麦克风

### 6.1 采集与混合

- **输入回调**（`MicCapture`）：多声道下混为单声道，线性插值重采样到输出采样率，写入 SPSC 环。
  蓝牙耳机通话模式（8/16 kHz）、手机 mic 软件（常见 44.1/48 kHz）都走这条换算。
- **输出回调**（`MicReader`）：从环读取，按麦克风增益叠加到所有输出声道。
  - 抖动缓冲：环中累积到约 15 ms 才开始出声，吸收两个设备的回调节奏差。
  - 漂移裁剪：积压超过目标 60 ms 时丢弃多余部分，延迟不会越积越大。
  - 欠载 / 设备断开：自动静音并重新缓冲。
- 静音只停止送出，电平表仍显示输入，方便确认麦克风在工作。
- 最终输出硬限幅到 [-1, 1]。

### 6.2 运行中切换设备（`mic.rs`）

麦克风来源经常变化（手机 mic 软件、蓝牙耳机随时连上/断开），所以输入流由专用麦克风线程持有：

- **切换**：`POST /api/mic/device` → `MicRequest::Select` → 线程打开新设备，**成功后**才替换旧流；
  失败返回 400，原设备保持不变。
- **换环**：每次打开新流都新建一个 SPSC 环，消费端经无锁通道交给输出回调；输出回调把旧环
  送回麦克风线程释放，实时线程不做大块内存释放。
- **断开检测**：输入流出错（`build_input` 的 `on_error`）时打标记；线程每秒巡检，发现断开就
  按原选择重连，状态显示「等待麦克风连接…」。蓝牙耳机重新连上后自动恢复。
- **跟随系统默认**：选择为「跟随系统默认」时，Windows 默认录音设备一变（如连上手机 mic），
  线程自动切过去。
- **输入设备检查**（`input_risk`，ADR-0006）：打开前按设备名判断，命中则拒绝打开，
  `GET /api/devices` 的输入列表也会排除。规则不限厂商：

  | 情况 | 判定 | 例子 |
  |---|---|---|
  | 名称含回录关键词 | 回录设备：会录进游戏等所有正在播放的声音 | `Stereo Mix`、`立体声混音`、`What U Hear`、Linux `Monitor of …` |
  | 与输出同名 | 反馈环 | VAC `Line 1 (Virtual Audio Cable)`、macOS `BlackHole 2ch` |
  | 括号内驱动名相同，且是虚拟设备 | 反馈环 | `CABLE Input/Output (VB-Audio Virtual Cable)`、`VoiceMeeter Input/Output (VB-Audio VoiceMeeter VAIO)` |
  | 无括号，去掉方向词后同名，且是虚拟设备 | 反馈环 | `… Virtual Source` / `… Virtual Sink` |

  「是虚拟设备」靠名称关键词（`virtual`、`cable`、`vb-audio`、`voicemeeter`、`vaio`、
  `blackhole`、`soundflower`、`loopback`）判断，因此同一块声卡的麦克风和扬声器
  （如 `Realtek(R) Audio`）不会被误判。名称启发式无法覆盖所有设备，命名特殊的虚拟线可能漏判。

### 6.3 设置持久化（`settings.rs`）

- 文件：数据目录下的 `settings.json`（数据目录见 §7）。
- 内容：`mic_device`（上次选的麦克风，`null` = 跟随系统默认）、`mic_gain_db`、`lan_remote`（手机遥控）。
- 启动时麦克风选择优先级：环境变量 `SOUNDBRIDGE_INPUT_DEVICE` > 上次的选择 > 系统默认。
- 静音状态不保存（由 `SOUNDBRIDGE_MIC_MUTED` 决定启动值）。

## 7. 音效库（`library.rs`）

- **数据目录**：`%APPDATA%\SoundBridge\`，可用 `SOUNDBRIDGE_DATA_DIR` 覆盖（测试用）。
  - `library.json`：`{ next_id, sounds: [SoundSpec...] }`，每次修改立即写盘（临时文件 + 改名，防止写一半损坏）。
  - `sounds/`：导入时复制进来的音频文件，命名 `<id>-<清理后的原文件名>`。
- **导入**（`POST /api/sounds/import`，multipart）：桌面拖拽、文件选择、手机上传共用。
  文件先写入 `sounds/`，再解码；解码失败返回 400 并删除刚写入的文件。默认名称 = 文件名去扩展名。
- **按路径添加**（`POST /api/sounds`）：文件留在原处，删除音效时不会删它。
- **启动加载**：逐个解码交给音频线程、绑定快捷键；文件丢失的音效保留在列表里并带 `error`，
  界面显示为不可播放的卡片。
- **id 不复用**：`next_id` 只增不减，即使导入失败或删除也不会回收，避免旧引用指错。
- 运行时信息（时长、加载错误）单独存放在内存，不写盘。

## 8. 全局热键（`hotkeys.rs`）

- 依赖 `global-hotkey`（Windows RegisterHotKey / macOS / Linux X11）。
- **Windows 必须有消息循环**：`WM_HOTKEY` 只投递到创建热键管理器的线程，热键线程每 10 ms
  用 `PeekMessageW/DispatchMessageW` 分发一次消息，否则热键永远不会触发。
- 绑定是请求-回复式的（`HotkeyCommand::Bind { reply }`）：先注册新键，成功后才注销旧键；
  快捷键无法识别、已被其它音效使用、或被其它程序占用时返回错误，API 返回 400，旧键保留。
- 快捷键字符串由 `HotKey::from_str` 解析，界面录制时直接用浏览器 `KeyboardEvent.code`
  拼成 `Ctrl+Alt+KeyQ` 这类格式；单独的字母/数字键会拦截全局打字，界面要求加修饰键。

## 9. 桌面 App（`apps/app`）

- **Tauri 2 外壳**：用系统 WebView2 显示界面，不打包浏览器内核。发布版 exe 约 9 MB，
  另需 `WebView2Loader.dll`（GNU 工具链动态加载）放在 exe 旁边，`build-app.ps1` 会一并复制。
- **进程内服务**：在单独线程的 tokio 运行时里跑 `soundbridge_server::run`，就绪后窗口加载
  `http://127.0.0.1:<端口>`。界面与浏览器版完全相同，通过 HTTP 访问，**不用 Tauri IPC**，
  因此不需要开放任何 Tauri 权限。
- **端口**：默认 8321，被占用（如浏览器版在运行）时自动换随机端口。
- **单实例**：`tauri-plugin-single-instance`，再次启动只会把已有窗口提到前台，避免两份音频输出。
- **拖拽导入**：关闭 Tauri 的文件拖放拦截（`disable_drag_drop_handler`），由网页处理 HTML5 拖放。
- **启动失败**：显示内置的错误页（`apps/app/loading/index.html`），错误信息经查询参数传入。
- **前端内嵌**：服务库用 `rust-embed` 把 `apps/web/dist` 编进可执行文件，App 不依赖工作目录；
  `index.html` 设为 `no-cache`，升级后不会看到旧界面。

## 10. 手机遥控

手机**不做音频处理**，只当「遥控器」：

```
手机浏览器 ──HTTP/局域网──▶ 电脑上的 SoundBridge ──命令──▶ 音频引擎 ──▶ 声卡/虚拟音频线
```

- **默认关闭**，只监听 `127.0.0.1`。设置里开启（`lan_remote`）后**重启生效**，改为监听 `0.0.0.0`，
  设置页显示手机访问地址（本机局域网 IP 通过查询路由表得到，不发送数据包）。
- `SOUNDBRIDGE_LAN=1/0` 可强制开关（覆盖设置）。
- 目前没有访问鉴权，只应在可信局域网开启。
- 绕开「移动端不能系统级注入麦克风」的平台限制。

## 11. 输出设备

- 启动时通过环境变量 `SOUNDBRIDGE_OUTPUT_DEVICE` 指定（包含匹配），否则用默认输出设备。
- `CpalBackend::resolve_output` 解析设备并取其原生配置（共享模式下的采样率/声道数），
  音效板与麦克风都按该采样率工作。
- 运行时切换输出设备留待后续。

## 12. 界面（`apps/web`）

- 无框架的 TypeScript：`api.ts`（类型 + 请求）、`icons.ts`（自绘 SVG 图标）、`main.ts`（界面逻辑）。
- 布局：左侧细导航栏（左下角设置）、中间音效卡片网格（搜索 / 全部停止 / 导入）、底部常驻麦克风条。
  窄屏（≤720 px，手机遥控）时隐藏导航栏，设置按钮移到工具栏。
- 深色为主，系统为浅色模式时自动切换浅色主题。
- 实时状态：每 100 ms 轮询 `/api/live`（播放中的音效、麦克风电平），只更新对应元素不重绘网格；
  每 1 s 轮询 `/api/mic`（设备状态）。
- 播放状态来自输出回调每次写入的 `PlayingSet`（固定 `MAX_INSTANCES` 个原子槽位），
  所以热键触发的音效也会在界面上高亮。

## 13. REST API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/app` | 版本、数据目录、输出设备、端口、手机遥控状态与地址 |
| POST | `/api/settings` | 修改设置 `{lan_remote?}` |
| GET | `/api/live` | `{playing: [id], mic_level, mic_connected}` |
| GET | `/api/devices` | 输入/输出设备列表（输入已排除反馈环设备与回录设备）+ 当前输出设备 |
| GET | `/api/mic` | 麦克风状态：选择、在用设备、采样率、是否连接、提示、音量、静音、电平 |
| POST | `/api/mic` | 设麦克风 `{gain_db?, muted?}`（音量会被记住） |
| POST | `/api/mic/device` | 切换麦克风 `{device: "名称" \| null}`，`null` = 跟随系统默认 |
| GET | `/api/sounds` | 列出音效（含 `file_name`、`duration_secs`、`error`） |
| POST | `/api/sounds/import` | 上传导入（multipart：`file`，可选 `name`；上限 200 MB） |
| POST | `/api/sounds` | 按本机路径添加 |
| PATCH | `/api/sounds/:id` | 编辑 `{name?, trigger_mode?, gain_db?, speed?, hotkey?}`；`hotkey: null` 清除，缺省不改 |
| DELETE | `/api/sounds/:id` | 删除（导入的文件一并删除） |
| POST | `/api/sounds/:id/trigger` · `/stop` | 触发 / 停止 |
| POST | `/api/stop-all` | 停止全部 |
| POST | `/api/sounds/:id/gain` · `/speed` | 旧接口，兼容保留（新界面用 PATCH） |

## 14. 数据模型（前端 JSON）

```jsonc
// 音效（GET /api/sounds）
{
  "id": 1,
  "name": "鼓掌",
  "file_name": "1-clap.mp3",
  "trigger_mode": "OneShot",   // OneShot | Loop | Exclusive | Overlap
  "gain_db": 0.0,
  "speed": 1.0,
  "hotkey": "Ctrl+Alt+KeyQ",   // 可选；null = 未绑定
  "duration_secs": 2.4,         // 加载失败时为 null
  "error": null                 // 加载失败原因，如文件被删除
}

// 麦克风（GET /api/mic）
{
  "selection": null,            // 用户选择；null = 跟随系统默认
  "device": "麦克风 (WO Mic Device)",
  "sample_rate": 48000,
  "connected": true,
  "note": null,                 // 未连接时的原因，如「等待麦克风连接…」
  "gain_db": 0.0,
  "muted": false,
  "level": 0.12                 // 0..1，增益后、静音前
}
```
