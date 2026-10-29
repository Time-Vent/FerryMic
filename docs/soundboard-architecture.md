# 音效板架构（Soundboard）

[English](en/soundboard-architecture.md) | **简体中文**

> 本文档描述「音效板」功能的技术架构：麦克风 + 音效混合、输出切换与试听、音效库、快捷键触发、
> 桌面 App（托盘、关闭询问、图标着色）、界面、手机遥控。

## 1. 定位

音效板是 FerryMic 的**核心产品形态**：采集真实麦克风，管理多个音效并绑定快捷键，
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
                    ├─ library.rs    音效库 = 磁盘文件夹（扫描、文件夹操作、.soundbridge.json、旧版迁移）
                    ├─ loader.rs     加载线程：后台预加载 + 按需加载；Player = 唯一触发入口
                    ├─ audio.rs      输出线程：SoundBoard 混音 + 试听分接 + 叠加麦克风 + 限幅；运行中切换输出；PlayingSet（含进度）
                    ├─ mic.rs        麦克风线程：自动选硬件麦克风/切换/断开重连、输入设备检查
                    ├─ monitor.rs    试听线程：只含音效的混音 → 第二个输出流（耳机 / 扬声器）
                    ├─ remote.rs     手机遥控：局域网监听（端口 + 1）、访问码、二维码，运行中开关
                    ├─ settings.rs   设置持久化（设备、试听、手机遥控、外观、窗口、音效库位置）+ 数据目录
                    ├─ i18n.rs       界面语言：tr! / trf! 生成给用户看的提示
                    ├─ hotkeys.rs    全局热键线程（Win32 消息循环）
                    ├─ virtual_mic.rs Linux 虚拟麦克风（pactl）
                    └─ （依赖）soundboard crate → audio-source（解码 + 共享 PCM + 变速 + 定位）
apps/web（Vite + TS，无框架）  界面，构建产物嵌入服务库
```

```
crates/soundboard/
  ├─ PlayMode       播放方式：{ looping, exclusive } 两个独立开关（旧 TriggerMode 仅用于读旧数据）
  ├─ SoundSpec      音效定义：名称/文件/播放方式/增益/变速/快捷键
  ├─ LoadedSound    已解码音效 = SoundSpec + 共享 PCM（控制侧解码）
  ├─ BoardCommand   控制命令：AddSound(LoadedSound) / Trigger / Stop / SetGain / SetSpeed / SetMode / Seek ...
  ├─ Instance       播放实例（克隆的 VarispeedSource + 增益/倍速/采样率换算比）
  └─ SoundBoard     音效板：管理定义与实例，fill 混音（实时线程独占）
```

## 3. 线程拓扑

| 线程 | 职责 | 实时约束 |
|---|---|---|
| tokio 线程池 | axum 服务器，REST API；解码、设备枚举、等待回复都在 `spawn_blocking` 中 | 无 |
| 加载线程 `soundbridge-loader` | 解码音效交给音频线程；触发请求优先于预加载 | 无 |
| 输出线程 `soundbridge-output` | 持有主输出流（cpal 流句柄不能跨线程）；处理切换输出请求 | 无 |
| 麦克风线程 `soundbridge-mic` | 持有输入流；处理切换请求；每秒巡检断开/默认设备变化 | 无 |
| 试听线程 `soundbridge-monitor` | 持有试听输出流；处理开关 / 换设备请求；主输出变化时重开 | 无 |
| 热键线程 `soundbridge-hotkeys` | 创建热键管理器 + Win32 消息循环；处理绑定请求，按下发 Trigger | 无 |
| cpal 输入回调线程 | 麦克风 → 下混单声道 → 重采样到输出采样率 → 写 SPSC 环 | **实时** |
| cpal 输出回调线程 | 消费命令(≤32) → SoundBoard::fill → 发布播放状态 → 试听分接 → 叠加麦克风 → 限幅 → 写输出 | **实时** |
| cpal 试听回调线程 | 从试听环读取 → 按采样率比插值 → 乘试听音量 → 写耳机 / 扬声器 | **实时** |
| （App）Tauri 主线程 | 原生窗口与 WebView2、托盘、图标着色；服务在单独线程的 tokio 运行时里 | 无 |

**数据流**：

```
麦克风(手机 mic / 蓝牙耳机 / USB) ─输入回调─▶ 单声道 + 重采样 ─SPSC 环─┐
                                                                     ▼
Web API / 全局热键 ──Player──▶（未加载则经加载线程解码）──BoardCommand──▶
    输出回调：SoundBoard 混音 ──┬──▶ + 麦克风 ─▶ 虚拟音频线（游戏里选它的另一端当麦克风）
                               └─试听环─▶ 试听回调 ─▶ 耳机 / 扬声器（只有音效）
```

**实时安全**：输出/输入回调不分配、不加锁、不做 IO。音频文件由加载线程解码
（`LoadedSound::load`），触发时实时线程只克隆 `Arc` 共享的 PCM；拖动进度只改实例的相位。

## 4. 播放方式

旧版的四种触发模式（点按 / 循环 / 独占 / 叠加）其实是两组互相独立的选择，现在拆成两个开关
`PlayMode { looping, exclusive }`：

| 开关 | 开 | 关 |
|---|---|---|
| `looping`（卡片上的双箭头图标，单次时带「1」） | 循环：按一下开始循环，再按停止 | 单次：每按一次从头播一遍，播完自动停止 |
| `exclusive`（卡片上的图标：独占 = 圆点，叠加 = 两个交叠的圆） | 独占：开始播放时停止其它所有音效 | 叠加：可与其它音效同时播放 |

两者可以组合（如「独占 + 循环」做背景音乐）。`BoardCommand::SetMode` 改设置时，正在播放的实例
就地更新循环标志，不打断播放。

- **兼容旧数据**：`.soundbridge.json` 里旧的 `trigger_mode` 读入时转换：`OneShot` / `Overlap` → 都关，
  `Loop` → 循环，`Exclusive` → 独占；下次写盘只写新的 `mode` 字段。

每个音效独立配置：播放方式 + 增益（dB）+ 倍速（varispeed，变速变调）。
同时播放的实例上限 `MAX_INSTANCES = 32`（预分配），满了替换最早的实例。

## 5. 混音（SoundBoard::fill）

- 遍历所有播放实例，每个实例 `VarispeedSource::fill` 到 scratch，按增益累加到输出。
- **采样率换算**：步进 = 用户倍速 × 源采样率 / 输出采样率，44.1 kHz 文件在 48 kHz 输出上不变调。
- **分块**：驱动单次要的帧数超过 scratch 容量时分块处理，不会越界。
- 实例源声道转换到输出声道（mono 复制 / stereo 直接 / 其它取交集）。
- 非循环实例播放完（exhausted）自动移除；循环实例 reset 继续。
- **进度与定位**：`for_each_playing` 回调（id, 进度 0..1），输出回调每次写入 `PlayingSet`
  （固定 `MAX_INSTANCES` 个原子槽位）；`BoardCommand::Seek` 把该音效所有实例的相位设到指定进度。
  同一音效多个实例时，界面显示最新实例的进度。

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
- **自动（默认）**：选择为「自动」时，系统默认录音设备是硬件麦克风就用它，否则用第一个硬件麦克风
  （「硬件」= 不是虚拟音频线、不是回录设备、不与当前输出构成反馈环）。默认设备一变（如连上手机 mic），
  线程自动切过去。
- **输出切换后**：输出线程发 `MicRequest::OutputChanged`，麦克风线程按新的采样率重开、重新做反馈环检查。
- **输入设备检查**（`input_risk`，ADR-0006）：打开前按设备名判断，命中则拒绝打开。
  `GET /api/devices` 的输入项带 `virtual` 标记，界面把虚拟设备单独分组。规则不限厂商：

  | 情况 | 判定 | 例子 |
  |---|---|---|
  | 名称含回录关键词 | 回录设备：会录进游戏等所有正在播放的声音 | `Stereo Mix`、`立体声混音`、`What U Hear`、Linux `Monitor of …` |
  | 与输出同名，且是虚拟设备 | 反馈环 | VAC `Line 1 (Virtual Audio Cable)`、macOS `BlackHole 2ch` |
  | 括号内驱动名相同，且是虚拟设备 | 反馈环 | `CABLE Input/Output (VB-Audio Virtual Cable)`、`VoiceMeeter Input/Output (VB-Audio VoiceMeeter VAIO)` |
  | 无括号，去掉方向词后同名，且是虚拟设备 | 反馈环 | `… Virtual Source` / `… Virtual Sink` |

  「是虚拟设备」靠名称关键词（`virtual`、`cable`、`vb-audio`、`voicemeeter`、`vaio`、
  `blackhole`、`soundflower`、`loopback`）判断，因此同一块声卡的麦克风和扬声器
  （如 `Realtek(R) Audio`）、蓝牙耳机同名的麦克风与听筒（如 `耳机 (AirPods)`）不会被误判。
  名称启发式无法覆盖所有设备，命名特殊的虚拟线可能漏判。

### 6.3 设置持久化（`settings.rs`）

- 文件：数据目录下的 `settings.json`。数据目录：Windows `%APPDATA%\FerryMic`、
  Linux `$XDG_DATA_HOME/FerryMic`（默认 `~/.local/share/FerryMic`）、
  macOS `~/Library/Application Support/FerryMic`，可用 `SOUNDBRIDGE_DATA_DIR` 覆盖（测试用）。
- 内容：
  - 设备：`mic_device`（`null` = 自动）、`mic_gain_db`、`output_device`（`null` = 系统默认）；
  - 试听：`monitor_enabled`（默认开）、`monitor_device`（`null` = 系统默认播放设备）、`monitor_gain_db`；
  - 手机遥控：`lan_remote`、`remote_token`（访问码，首次启动生成）；
  - 外观：`theme_mode`（system / dark / light）、`accent`（`#rrggbb`）、`tint_app_icon`（默认开）、
    `language`（`system` / `zh-CN` / `en`，默认 `system`）；
  - 窗口：`close_action`（`Ask` / `Tray` / `Quit`，默认 `Ask`）；
  - 其它：`library_dir`（`null` = 数据目录下的 `sounds/`）、`auto_virtual_mic`（Linux）；
  - 新音效默认设置：`sound_defaults`（`{looping, exclusive, gain_db, speed}`，默认独占、单次、0 dB、1.0×）；
  - 音效资源：`custom_sites`（用户添加的网站 `[{name, url}]`）。
  旧文件缺少的字段自动取默认值。
- **改名迁移**（`migrate_legacy_data_dir`）：启动时若旧数据目录 `SoundBridge` 存在而 `FerryMic` 不存在，整个改名过去；
  `library_dir` 指向旧目录内部时用 `rebase_path` 换到新目录。设置了 `SOUNDBRIDGE_DATA_DIR` 时不迁移。
- 启动时麦克风选择优先级：环境变量 `SOUNDBRIDGE_INPUT_DEVICE` > 上次的选择 > 自动。
  输出：`SOUNDBRIDGE_OUTPUT_DEVICE` > 上次的选择 > 系统默认。
- 静音状态不保存（由 `SOUNDBRIDGE_MIC_MUTED` 决定启动值）。

## 7. 音效库（`library.rs` + `loader.rs`）

### 7.1 磁盘文件夹即音效库

- **根目录**由设置决定；根目录下的音频文件就是音效，**子文件夹就是分类**（最多 8 层，最多 1 万个文件，
  以 `.` 开头的项忽略）。支持 WAV / MP3 / FLAC / OGG / M4A / AAC。
- **每个音效的设置**存在根目录的 `.soundbridge.json`：`{ version: 2, next_id, sounds: { "相对路径": {id, mode: {looping, exclusive}, gain_db, speed, hotkey} } }`，
  按 `/` 分隔的相对路径索引，因此整个文件夹可以搬走 / 同步。写盘用临时文件 + 改名。
- **名称 = 文件名去扩展名**：App 里改名就是改文件名，移动就是移动文件，设置随之迁移；
  在资源管理器里改名 / 移动则视为删除 + 新增，设置会丢失。
- **扫描**（启动、`POST /api/library/rescan`、窗口重新获得焦点时）：同步条目与设置，返回新增 / 删除的 id，
  由 API 层据此卸载 / 预加载音效、解绑 / 绑定快捷键。
- **新音效的初始设置**：扫描时第一次见到的文件按 `SoundDefaults`（来自设置 `sound_defaults`，修改后经
  `Library::set_defaults` 立即生效）建设置；已有音效不受影响。设置里「应用到所有现有音效」由界面逐个 PATCH 完成，
  快捷键、名称、文件夹不变。
- **id**：记在设置文件里，重启后不变（快捷键靠它）；新文件分配新 id，`next_id` 只增不减；
  手改设置文件造成重复 id 时自动重新分配。
- **文件夹操作**：新建、重命名 / 移动（不能移进自己的子文件夹）、删除（只允许删除不含音效的文件夹）。
  所有相对路径经 `clean_rel` 清理，`..`、绝对路径、非法字符都会被去掉，防止路径穿越。
- **导入**（`POST /api/sounds/import`，multipart：`file` + 可选 `folder`）：写入对应文件夹，重名自动编号
  （`名称 (2).mp3`），再解码校验；失败则删除刚写入的文件并返回 400。
- **删除**：文件移到系统回收站（`trash` crate，Linux 为 freedesktop 回收站），不支持时直接删除。
- **修改位置**（`POST /api/library/location`）：可选「把现有文件一起移过去」（同盘改名，跨盘复制后删除，
  目标已有同名项时报错）；切换后卸载旧库全部音效、解绑快捷键，再加载新库。
- **旧版迁移**（`migrate_v1`）：首次启动时若数据目录有旧版 `library.json`，把文件改名为「名称.扩展名」
  放进根目录，设置（含 id、快捷键）写入 `.soundbridge.json`，原文件改名为 `library.v1.json.bak`。

### 7.2 加载（`loader.rs`）

音效库可能指向很大的文件夹，全部解码进内存不现实（3 分钟立体声 ≈ 66 MB）：

- **加载线程**按顺序预加载，累计解码量超过 `PRELOAD_BUDGET_BYTES`（768 MB）后停止预加载。
- 没加载的音效**第一次触发时**再解码，解码完立刻播放；触发请求走优先队列，不排在预加载后面。
- `Player::trigger` 是唯一的触发入口，Web API 与全局热键都走它，所以热键也能触发未加载的音效。
- 改音量 / 倍速 / 播放方式时，已加载的音效同步发命令给音频线程；未加载的下次加载时直接读到最新设置。
- **切换输出设备时**新的 `SoundBoard` 是空的，`Player::reinstall_all` 把缓存里已解码的音效重新装进去
  （`LoadedSound` 克隆只复制 `Arc`，不重新解码）。
- 运行时信息（时长、是否已加载、加载错误）只在内存里，界面据此显示时长；有未加载的音效时界面每 3 秒刷新一次。

## 8. 全局热键（`hotkeys.rs`）

- 依赖 `global-hotkey`（Windows RegisterHotKey / macOS / Linux X11）。
- **Windows 必须有消息循环**：`WM_HOTKEY` 只投递到创建热键管理器的线程，热键线程每 10 ms
  用 `PeekMessageW/DispatchMessageW` 分发一次消息，否则热键永远不会触发。
- 绑定是请求-回复式的（`HotkeyCommand::Bind { reply }`）：先注册新键，成功后才注销旧键；
  快捷键无法识别、已被其它音效使用、或被其它程序占用时返回错误，API 返回 400，旧键保留。
- 快捷键字符串由 `HotKey::from_str` 解析，界面录制时直接用浏览器 `KeyboardEvent.code`
  拼成 `Ctrl+Alt+KeyQ` 这类格式；单独的字母/数字键会拦截全局打字，界面要求加修饰键。

## 9. 桌面 App（`apps/app`）

- **Tauri 2 外壳**：用系统 WebView（Windows 为 WebView2，Linux 为 WebKitGTK）显示界面，不打包浏览器内核。
  Windows 发布版 exe 约 9 MB，另需 `WebView2Loader.dll`（GNU 工具链动态加载）放在 exe 旁边，
  `build-app.ps1` 会一并复制；Linux 用 `build-app.sh` 构建。
- **进程内服务**：在单独线程的 tokio 运行时里跑 `soundbridge_server::run`，就绪后窗口加载
  `http://127.0.0.1:<端口>`。界面与浏览器版完全相同，通过 HTTP 访问，**不用 Tauri IPC**，
  因此不需要开放任何 Tauri 权限。
- **端口**：默认 8321，被占用（如浏览器版在运行）时自动换随机端口。
- **单实例**：`tauri-plugin-single-instance`，再次启动只会把已有窗口提到前台，避免两份音频输出。
- **拖拽导入**：关闭 Tauri 的文件拖放拦截（`disable_drag_drop_handler`），由网页处理 HTML5 拖放。
- **启动失败**：显示内置的错误页（`apps/app/loading/index.html`），错误信息经查询参数传入。
- **托盘**：`TrayIconBuilder` 建托盘图标，菜单「打开 FerryMic / 退出」，左键单击恢复窗口。
- **关闭询问**：拦截 `CloseRequested`，按设置 `close_action` 处理：
  - `Tray`：隐藏窗口；`Quit`：退出；
  - `Ask`（默认）：`Control::request_close` 置位，界面轮询 `/api/live` 看到 `close_prompt` 后弹出网页内的对话框
    （「最小化到托盘 / 退出 / 取消」+「记住我的选择」），回答经 `POST /api/window` → `on_window_action` 钩子回到外壳。
    用网页对话框而不是原生对话框，样式与主题一致；对话框未响应时（如界面卡死）再点一次 × 直接退出。
  - 其他窗口（音效资源预览窗口）点关闭只关闭它自己。
- **图标着色**：主题色变化时（`on_settings` 钩子），把内置图标 `icons/icon.png` 的像素按
  「主题色色相 − 原图色相 246.1°」旋转色相，跳过低饱和度（白色）像素，设为窗口与托盘图标。
  设置 `tint_app_icon` 关闭时恢复原图标。
- **钩子**：服务库不依赖 Tauri，外壳通过 `ServerConfig.hooks`（`on_settings`、`on_window_action`、`on_open_url`）接收事件。
- **音效资源预览窗口**：`open_browse_window` 打开或复用 `browse` 窗口；`on_download` 把音频下载改存到音效库「下载」文件夹；
  图标着色对所有窗口生效。
- **前端内嵌**：服务库用 `rust-embed` 把 `apps/web/dist` 编进可执行文件，App 不依赖工作目录；
  `index.html` 设为 `no-cache`，升级后不会看到旧界面。

## 10. 手机遥控

手机**不做音频处理**，只当「遥控器」：

```
手机浏览器 ──HTTP/局域网──▶ 电脑上的 FerryMic ──命令──▶ 音频引擎 ──▶ 声卡/虚拟音频线
```

- **主界面始终只监听 `127.0.0.1`**。手机遥控是**单独的监听**（`0.0.0.0:主端口+1`，被占用时换随机端口），
  由 `RemoteServer` 在运行中启动 / 优雅关闭，**不用重启**；左侧导航栏的手机按钮一键开关。
- 两个监听共用同一个 axum `Router`（同一份 `AppState`），手机端额外套一层 `require_token` 中间件：
  - 访问码 `remote_token` 存在设置里；网址形如 `http://192.168.x.x:8322/?t=<访问码>`，
    首次访问后写入 Cookie `sb_token`，之后不用带参数；没有访问码返回 401 提示页。
  - 「重新生成访问码」后旧码立即失效。
  - 电脑端专属接口（`/api/remote`、`/api/settings`、`/api/pick-folder`、`/api/library/location`、
    `/api/library/open`、`/api/output`、`/api/monitor`、`/api/open-url`）对手机只允许 GET。
- 面板里显示网址和二维码（`qrcode` crate 生成 SVG，`GET /api/remote/qr.svg`）；
  本机局域网 IP 通过查询路由表得到，不发送数据包。
- `SOUNDBRIDGE_LAN=1/0` 可强制开关（覆盖设置）。
- 绕开「移动端不能系统级注入麦克风」的平台限制。


## 11. 输出设备、试听与虚拟麦克风

### 11.1 输出音频线路（主输出）

- 优先级：`SOUNDBRIDGE_OUTPUT_DEVICE`（包含匹配）> 设置 `output_device` > 系统默认。
- `CpalBackend::resolve_output` 解析设备并取其原生配置（共享模式下的采样率/声道数），
  音效板与麦克风都按该采样率工作。
- **分类**（`describe_output`）：输入设备里能找到与它构成反馈环的另一端（`game_mic_for`），
  或名称是虚拟设备 → `Virtual`，并给出其他软件里该选的麦克风（如 `CABLE Input` → `CABLE Output`）；
  否则是 `Speaker`。界面底部显示「其他软件的麦克风选：…」，输出到扬声器时改为黄色警告。
- **运行中切换**（`POST /api/output`）：输出线程收到 `OutputRequest::Select`，先丢掉旧流，
  新建 `SoundBoard` 并 `reinstall_all` 已解码的音效，再打开新设备；失败则重开原设备并返回 400。
  成功后通知麦克风线程（`OutputChanged`）与试听线程（`MainChanged`），并记住选择。

### 11.2 试听（`monitor.rs`）

输出到虚拟音频线时，自己听不到音效。试听把**只含音效**的混音再放一份到耳机 / 扬声器：

- 输出回调里 `SoundBoard::fill` 之后、叠加麦克风之前，`MonitorTap` 把当前缓冲（下混 / 扩展为立体声）
  写进 SPSC 环；试听线程的第二个输出流用 `MonitorReader` 读出。
- **不含自己的麦克风**：自己说话的延迟回声很难受。
- **两台设备采样率不同**：按 `主输出采样率 / 试听采样率` 线性插值；先积累约 20 ms 再出声，
  积压超过 60 ms 就丢弃多余部分，两个时钟的漂移不会让延迟越积越大。
- **不需要试听时自动跳过**：主输出本身就是扬声器，或试听设备与主输出是同一台设备时，状态显示原因、不打开。
- 设置：开关（默认开）、设备（默认 = 系统默认播放设备）、音量；底部耳机按钮一键开关。
- 环的交接与麦克风相同：新环经通道交给输出回调，旧环送回控制侧释放。

### 11.3 Linux 自动虚拟麦克风（`virtual_mic.rs`）

```
FerryMic 输出 ──▶ 虚拟输出 soundbridge_mix（module-null-sink，描述 FerryMic-Mix）
                        └─ monitor ──▶ 虚拟麦克风 soundbridge_mic（module-remap-source，描述 FerryMic-Microphone）
                                             └─▶ 其他软件里选它当麦克风
```

- 条件：Linux、未指定输出设备（环境变量与设置都没有）、设置 `auto_virtual_mic` 为真（默认）。
- 用 `pactl load-module` 创建（PipeWire 的 pipewire-pulse 与 PulseAudio 都支持），服务结束时
  `unload-module`；启动时先查 `pactl list short modules`，上次异常退出遗留的模块直接接管复用。
- cpal 在 Linux 走 ALSA，无法按名字选 PulseAudio 设备，所以在打开音频流**之前**设置环境变量
  `PULSE_SINK` / `PIPEWIRE_NODE` = `soundbridge_mix`，让本进程的播放进入虚拟输出（只影响播放）。
- 失败（如没有 `pactl`）不致命：设置页显示原因，音效照常输出到默认设备。
- 解析 `pactl` 输出的函数与平台无关，在 Windows 上也有单元测试；Linux 版已在 CI 编译并通过测试，但整体流程尚未在真实 Linux 桌面上实测。
- Windows / macOS 没有系统自带的虚拟麦克风接口，仍需 VB-Cable / BlackHole 等第三方虚拟音频线（见 ADR-0004）。

## 12. 界面（`apps/web`）

- 无框架的 TypeScript：`api.ts`（类型 + 请求）、`icons.ts`（自绘 SVG 图标）、`i18n.ts`（多语言）、`sites.ts`（内置音效网站）、`main.ts`（界面逻辑）。
- **布局**：左侧细导航栏（音效库、音效资源、手机遥控开关——右上角信号波亮起表示已开启、使用说明、设置）、文件夹栏（文件夹树 + 新建 / 刷新 + 音效库位置）、
  中间音效卡片网格（搜索 / 全部停止 / 导入）、底部常驻条：你的麦克风（静音 / 电平 / 音量）、
  试听按钮（耳机）、输出目标（「其他软件的麦克风选：CABLE Output」，点击打开设置的「音频链路」）。
- **措辞**：界面不点名具体软件（游戏、Discord 等），统一说「输出音频线路」「其他软件」。
  窄屏（≤720 px，手机遥控）时隐藏两侧栏，文件夹变成顶部横向标签，设置按钮移到工具栏。
- **文件夹**：选中文件夹显示其中（含子文件夹）的音效；「⋯」菜单新建子文件夹 / 重命名 / 删除；
  卡片可拖到文件夹上移动；从系统拖入文件时，放在某个文件夹上就导入到那里，否则导入到当前文件夹。
- **卡片**：上半部分为播放按钮、名称、时长 / 快捷键 / 所在文件夹（单行）；中间是进度条
  （始终占位、只在播放时显示，保证卡片等高，按住拖动跳转）；底部是快捷控件：
  - 循环 / 单次：音乐播放器式的双箭头图标，点一下切换；
  - 独占 / 叠加：同样的图标按钮（圆点 / 两个交叠的圆），点一下切换；
  - 这两组开关没有「更好」的一方，所以透明无边框、两种状态同色，只换图标；
  - 音量：点开竖向滑条（停手后保存）；
  - 倍速：纯文字，点一下切到下一档预设，右键回上一档。
  「⋯」打开完整编辑（名称即文件名、文件夹、循环、独占、音量、倍速、快捷键、删除）。
- **设置**：外观（含「App 图标跟随主题色」、语言）、音效库位置、新音效的默认设置、音频链路（链路示意图、你的麦克风、
  输出音频线路、试听、Linux 虚拟麦克风）、窗口（关闭按钮行为）、使用说明。
  只在桌面 App 里有意义的项（图标、窗口）带 `.app-only`，浏览器 / 手机里隐藏（`/api/app` 的 `is_app`）。
- **音效资源**（`sites.ts`）：左侧地球按钮切换到「音效资源」页（隐藏文件夹栏与音效网格，底部条保留）。
  - 内置网站列表在前端（名称、网址、一句话介绍、授权概括，介绍与授权经 `t()` 翻译）；用户添加的存服务端设置 `custom_sites`
    （最多 50 个，名称 1–60 字，网址必须是 http(s)），添加时先问网址（没写协议补 `https://`）再问名称（默认取主机名）。
  - 卡片按钮：「预览」（眼睛图标）、「在浏览器中打开」（地球图标）。页面右上角搜索框按名称、网址、介绍、授权筛选。
  - 打开：桌面 App 调 `POST /api/open-url` → `on_open_url` 钩子 → 外壳打开（或复用）标签为 `browse` 的预览窗口；
    `external` 时用系统浏览器（Windows `rundll32 url.dll,FileProtocolHandler`）。浏览器版 / 手机直接开新标签页。
    `/api/open-url` 属于电脑端专属接口，手机不能让电脑开窗口。
  - **下载直接入库**：预览窗口的 `on_download` 钩子检查文件名，是音频格式就把保存位置改到音效库的「下载」
    （英文界面为 `Downloads`）文件夹；该文件夹里已有同名文件时取消下载（不再产生副本），取消引起的「失败」事件不提示。
    其它文件照常进系统下载目录。
  - **预览窗口 = 自绘标题栏 + 网站**：系统标题栏放不进自定义按钮，所以预览窗口不用系统边框（`decorations(false)`），
    用 Tauri 多 webview（`unstable` 特性）放两块：上面是本地页面 `apps/app/loading/titlebar.html`
    （网站名、当前地址、地球按钮、最小化 / 最大化 / 关闭），下面是网站。窗口大小变化时 `layout_browse_window` 重新摆放。
    - 标题栏只开放拖动与这几个窗口按钮的权限（`apps/app/capabilities/preview-titlebar.json`，只作用于 `browse-bar`）；
      网站那块没有任何 Tauri 权限。拖动、双击最大化、Aero Snap、从边缘调整大小照常可用；
      Win11 悬停最大化按钮弹出的布局菜单没有。
    - 地球按钮跳到保留地址 `https://ferrymic-open.invalid/`，标题栏的 `on_navigation` 拦下后读取网站当前网址，
      用系统浏览器打开。网站内跳转时 `on_page_load` 更新标题栏里的地址。
  - **下载提示**（`apps/app/preview/preview.js`，注入网站页面，Shadow DOM 不受网站样式影响）：下载开始 / 完成 / 跳过 / 失败时
    外壳用 `eval` 调 `__ferryToast` 在网站区域右上角提示，下载完成后通知主窗口（`ferrymic:rescan` 事件）立即重新扫描。回到主窗口时自动重新扫描，新文件出现在列表里。
    这样不用为每个网站写解析逻辑，网站自己的下载按钮就是「一键入库」。
  - 预览窗口加载的是第三方网页：不开放任何 Tauri 权限，与主界面一样只用 HTTP。
- **多语言**（`i18n.ts`）：中文是源语言，英文按中文原文查表（`t('中文 {参数}', {参数})`）。
  - HTML 静态文字（文本节点与 `title` / `placeholder` / `aria-label` / `alt`）由 `translateDom` 整段替换，
    原文记在 `WeakMap` 里，切回中文时恢复；含排版的说明块用 `data-i18n-html="键"` 整块替换。
  - 查不到的文字显示中文原文，漏翻不会出错。设备名、文件夹名等用户数据不翻译。
  - 语言偏好存在服务端设置里；`/api/app` 返回解析后的 `language_effective`（`system` 由后端按操作系统语言决定，
    中文系统用中文，其它用英文），界面以它为准；`localStorage` 记住上次的语言，启动时避免中英闪烁。
  - 切换后静态文字立即替换，动态内容（文件夹、卡片、底部条、设置里的设备列表）重新渲染。
  - 后端（`i18n.rs`）：原子量记录当前语言，`tr!("中", "en")` / `trf!("中 {x}", "en {x}")` 生成给用户看的提示与错误；
    日志仍是中英双语。桌面 App 收到 `on_settings` 钩子后重建托盘菜单。
  - 增加语言：在 `i18n.ts` 加一张词典、在 `i18n.rs` 的宏里加一个分支，并加入 `LANGUAGES`。
- **手机遥控面板**：开关、二维码、网址、重新生成访问码；未开启时点导航栏按钮会直接开启。
- **外观**：深色 / 浅色 / 跟随系统；主题色 7 种预设 + 自定义取色。脚本由主题色算出
  `--accent`、`--accent-2`（色相偏移 35° 的渐变第二色）、`--accent-soft` / `--accent-glow` / `--accent-line`
  （不同透明度），写到 `<html>` 上；同时存进 `localStorage`，下次启动在页面渲染前先套用，避免闪烁。
  设置本身保存在服务端 `settings.json`（App 端口可能变化，`localStorage` 不可靠）。
- **使用说明**：设置里内置「基础操作」与「各平台接入其他软件」，当前平台会标出「当前系统」；
  导航栏的「?」按钮直接打开并滚动到这里。
- **实时状态**：每 100 ms 轮询 `/api/live`（播放中的音效与进度、麦克风电平、是否要弹关闭询问），只更新对应元素不重绘网格；
  每 1 s 轮询 `/api/mic`（设备状态）。播放状态来自输出回调每次写入的 `PlayingSet`，
  所以热键触发的音效也会高亮并显示进度。
- **同步磁盘变化**：窗口重新获得焦点时自动重新扫描（2 秒内最多一次），左侧也有「刷新」按钮。

## 13. REST API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/app` | 版本、平台、数据目录、音效库位置、输出设备、端口、外观、手机遥控、`is_app`、`tint_app_icon`、`close_action`、`language` / `language_effective`、`sound_defaults`、`custom_sites`、虚拟麦克风状态 |
| POST | `/api/settings` | 修改设置 `{theme_mode?, accent?, auto_virtual_mic?, tint_app_icon?, close_action?, language?, sound_defaults?, custom_sites?}`（改后调用 `on_settings` 钩子） |
| POST | `/api/window` | 回答关闭询问 `{action: "tray" \| "quit" \| "cancel", remember}` |
| POST | `/api/open-url` | 打开网址 `{url, title?, external}`；只允许 http(s)；App 里默认用预览窗口 |
| POST | `/api/pick-folder` | 在电脑上弹出系统「选择文件夹」对话框（`rfd`），返回 `{path}` |
| GET | `/api/live` | `{playing: [{id, progress}], mic_level, mic_connected, close_prompt}` |
| GET | `/api/devices` | `{outputs: [{name, is_default, kind, game_mic}], inputs: [{name, is_default, virtual}], current}` |
| POST | `/api/output` | 切换主输出 `{device: "名称" \| null}`，返回新的输出信息；失败 400 并恢复原设备 |
| GET / POST | `/api/monitor` | 试听状态 / 设置 `{enabled?, device?, gain_db?}`（`device: null` = 系统默认） |
| GET / POST | `/api/remote` | 手机遥控 `{enabled, url, port, error}` / 开关 `{enabled?, regenerate?}` |
| GET | `/api/remote/qr.svg` | 当前手机网址的二维码（SVG） |
| GET | `/api/mic` | 麦克风状态：选择、在用设备、采样率、是否连接、提示、音量、静音、电平 |
| POST | `/api/mic` | 设麦克风 `{gain_db?, muted?}`（音量会被记住） |
| POST | `/api/mic/device` | 切换麦克风 `{device: "名称" \| null}`，`null` = 自动 |
| GET | `/api/library` | `{root, folders: [相对路径], sounds: [...]}` |
| POST | `/api/library/rescan` | 重新扫描磁盘并同步加载 / 快捷键 |
| POST | `/api/library/location` | 修改音效库位置 `{path, move_contents}` |
| POST | `/api/library/open` | 在系统文件管理器中打开音效库 |
| POST | `/api/folders` | 新建文件夹 `{path}` |
| PATCH | `/api/folders` | 重命名 / 移动 `{from, to}`（`to` 为新的完整相对路径） |
| DELETE | `/api/folders` | 删除不含音效的文件夹 `{path}`（移到回收站） |
| GET | `/api/sounds` | 列出音效 |
| POST | `/api/sounds/import` | 上传导入（multipart：`file`，可选 `folder`；上限 200 MB） |
| PATCH | `/api/sounds/:id` | 编辑 `{name?, folder?, looping?, exclusive?, gain_db?, speed?, hotkey?}`；`name` 改文件名，`folder` 移动文件，`hotkey: null` 清除，缺省不改 |
| DELETE | `/api/sounds/:id` | 删除（文件移到回收站） |
| POST | `/api/sounds/:id/trigger` · `/stop` | 触发（未加载则先加载）/ 停止 |
| POST | `/api/sounds/:id/seek` | 跳转 `{position: 0..1}` |
| POST | `/api/stop-all` | 停止全部 |

## 14. 数据模型（前端 JSON）

```jsonc
// 音效（GET /api/library 的 sounds 项）
{
  "id": 1,
  "name": "鼓掌",               // 文件名去扩展名
  "folder": "游戏/整活",        // 相对音效库根目录；根目录为 ""
  "file_name": "鼓掌.mp3",
  "looping": false,             // 循环（否 = 单次）
  "exclusive": false,           // 独占（否 = 叠加）
  "gain_db": 0.0,
  "speed": 1.0,
  "hotkey": "Ctrl+Alt+KeyQ",   // 可选；null = 未绑定
  "duration_secs": 2.4,         // 加载后才有
  "loaded": true,               // 是否已解码进内存
  "error": null                 // 加载失败原因
}

// 麦克风（GET /api/mic）
{
  "selection": null,            // 用户选择；null = 自动（优先硬件麦克风）
  "device": "麦克风 (WO Mic Device)",
  "sample_rate": 48000,
  "connected": true,
  "note": null,                 // 未连接时的原因，如「等待麦克风连接…」
  "gain_db": 0.0,
  "muted": false,
  "level": 0.12                 // 0..1，增益后、静音前
}

// 试听（GET /api/monitor）
{
  "enabled": true,
  "selection": null,            // null = 系统默认播放设备
  "device": "耳机 (AirPods)",   // 实际打开的设备
  "active": true,               // 是否正在试听
  "note": null,                 // 没生效的原因，如「输出已经是扬声器 / 耳机，不需要试听」
  "gain_db": 0.0
}
```
