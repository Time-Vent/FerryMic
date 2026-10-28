# SoundBridge 代码走读（学习用）

> 本文档逐个解释各 crate、模块、结构体、trait 与关键函数的用途，便于学习理解。
> 每个条目给出：它在哪、它是什么、它为什么这样设计、关键方法怎么用。

## 目录

1. [总览](#1-总览)
2. [audio-types：纯数据类型](#2-audio-types纯数据类型)
3. [platform-api：能力契约](#3-platform-api能力契约)
4. [dsp：定长处理器](#4-dsp定长处理器)
5. [audio-graph：处理链](#5-audio-graph处理链)
6. [audio-source：音频源与变速](#6-audio-source音频源与变速)
7. [audio-engine：引擎三执行器与时钟](#7-audio-engine引擎三执行器与时钟)
8. [application：装配层](#8-application装配层)
9. [audio-cpal：硬件后端](#9-audio-cpal硬件后端)
10. [desktop：CLI 壳](#10-desktopcli-壳)
11. [soundboard：音效板核心](#11-soundboard音效板核心)
12. [server：音效板服务库](#12-server音效板服务库)
13. [app：桌面 App（Tauri 外壳）](#13-app桌面-apptauri-外壳)

---

## 1. 总览

整个项目是一个 Cargo workspace，遵循「**分层 + 单向依赖**」：

```
apps/desktop (CLI 壳)
   └─ application (装配：把各层接起来)
        ├─ audio-engine (逻辑执行器 + 时钟)
        │    ├─ audio-graph (处理链)
        │    │    └─ dsp (增益/限幅/均衡)
        │    ├─ audio-source (文件/变速)
        │    └─ platform-api (能力 trait)
        ├─ audio-cpal (cpal 硬件实现)
        └─ audio-source
             └─ audio-types (纯数据，最底层)
```

**为什么要分层？** 实时音频对「不阻塞、不分配」极其敏感。分层后，实时路径只触碰最底层
的纯数据和预分配缓冲，业务逻辑（设备枚举、文件解码）被隔离在独立线程。契约（trait）与
实现（cpal）分离，未来换硬件后端或加平台不用改引擎。

---

## 2. audio-types：纯数据类型

**位置**：`crates/audio-types/src/lib.rs`
**定位**：唯一无内部依赖的「叶子」crate。所有模块之间传的都是这些纯数据，不传句柄、不传锁。

### 结构体/枚举

| 类型 | 含义 |
|---|---|
| `SampleRate(u32)` | 采样率 Hz，如 `48000` |
| `ChannelCount(u16)` | 声道数，1=单声道 2=立体声 |
| `FrameCount(usize)` | 帧数（一帧=每个声道一个采样） |
| `StreamConfig` | 采样率+声道+缓冲帧数的组合，是打开流的核心参数 |
| `SampleFormat` | 采样格式（F32 / I16），引擎内部统一 F32 |
| `DeviceId(String)` | 设备不透明唯一标识 |
| `DeviceInfo` | 设备描述（名称、是否默认、支持的输入/输出配置） |
| `EngineEvent` | 引擎事件（欠载、漂移警告、设备移除等） |
| `Gain(f32)` / `PlaySpeed(f32)` | 带「钳制」语义的增益/倍速包装 |

### 关键方法

- `Gain::clamped()`：把增益钳到 `0..64`，NaN 归零 —— 防御脏参数进实时路径。
- `PlaySpeed::clamped()`：把倍速钳到 `0.0625..16`，非法值回退到 1.0。

**设计点**：`StreamConfig`、`Gain` 都是 `Copy`，跨线程传递零成本、无堆分配。

---

## 3. platform-api：能力契约

**位置**：`crates/platform-api/src/lib.rs`
**定位**：只定义 trait 和错误类型，不写任何平台代码。这是「契约先于实现」原则的落地。

### trait

| trait | 职责 |
|---|---|
| `AudioBackend` | 枚举设备 + 打开输入/输出流（核心） |
| `OutputStreamHandle` | 输出流句柄，`pause`/`resume`，drop 停止 |
| `InputStreamHandle` | 输入流句柄 |
| `VirtualMic` | 虚拟麦克风（**预留**） |
| `SystemAudioLoopback` | 系统声音回环（**预留**） |
| `PermissionGate` | 权限门槛（**预留**） |

### 关键签名

```rust
fn open_output<F>(&self, config: &StreamConfig, callback: F)
    -> Result<Box<dyn OutputStreamHandle>, PlatformError>
where F: FnMut(&mut [f32]) + Send + 'static;
```

- `callback` 是实时回调，由后端在**音频线程**调用。`Send + 'static` 表示它可跨线程、生命周期无限。
- 返回 `Box<dyn OutputStreamHandle>`：用 trait 对象解耦具体后端类型。

### `PlatformError`

用 `thiserror` 派生，覆盖设备不存在、配置不支持、权限、回环反馈检测等。注意：实时路径
**不得**构造它（它携带堆分配 String）。

---

## 4. dsp：定长处理器

**位置**：`crates/dsp/src/`
**定位**：所有「同帧数」的处理器。变速（变帧数）不在这里，在 audio-source。

### `FixedProcessor` trait（`fixed.rs`）

```rust
pub trait FixedProcessor: Send {
    fn process(&mut self, input: &[f32], output: &mut [f32], frames: usize);
    fn reset(&mut self);
    fn latency_samples(&self) -> usize { 0 }
    fn set_param(&mut self, id: u32, value: f32);
    fn name(&self) -> &'static str;
}
```

- `process`：同帧数处理，输入输出都是交错样本（frames × channels）。
- `reset`：清空延迟线/包络（设备切换、seek 时用）。
- `latency_samples`：处理器引入的固定延迟（默认 0）。
- `set_param`：参数下发，`id` 用 `param` 模块里的常量。

### 处理器实现

**`Gain`（`gain.rs`）**：增益，带一阶平滑消除 zipper 噪声。
- `set_gain_db(db, smoothing_samples)`：`smoothing_samples=0` 立即生效，否则平滑过渡。
- 关键：`advance()` 每采样逼近目标值，避免增益跳变产生咔嗒声。

**`Limiter`（`limiter.rs`）**：峰值限幅，防削波。
- 硬限幅 + 指数释放包络：超阈值立即压低，之后按 `release_coeff` 向 1 恢复。
- `envelope` 是跨 buffer 状态，`reset` 恢复为 1。

**`Biquad`（`biquad.rs`）**：双二阶均衡，RBJ cookbook 系数。
- `BiquadParams::rbj(kind, fs, freq, q, gain_db)`：生成归一化系数。
- 延迟线 `x1/x2/y1/y2` 按声道保存，跨 buffer 保持 —— 这是「有状态处理器」的典型。

**`db_to_linear` / `linear_to_db`**：dB 与线性幅度互转，全项目共用。

---

## 5. audio-graph：处理链

**位置**：`crates/audio-graph/src/lib.rs`
**定位**：把多个 `FixedProcessor` 串成顺序链（第一阶段最简；第二阶段扩展为 DAG）。

### `ProcessingGraph`

- `new(capacity_samples)`：预分配两块 scratch 缓冲（双缓冲）。
- `push_processor(p)`：追加节点（实时路径建立前调用）。
- `process(input, output, frames, channels)`：核心 —— 双缓冲 ping-pong。

**双缓冲原理**：节点 0 读 `scratch_a` 写 `scratch_b`，节点 1 读 `scratch_b` 写 `scratch_a`，
交替进行，避免每个节点都分配新缓冲。最终结果在最后一个节点写的那块缓冲里。

**为什么 `process` 不分配？** 两块缓冲在 `new` 时一次性分配好，之后只借用切片，实时安全。

---

## 6. audio-source：音频源与变速

**位置**：`crates/audio-source/src/`

### `VariableRateSource` trait（`lib.rs`）

```rust
pub trait VariableRateSource: Send {
    fn fill(&mut self, out: &mut [f32], frames: usize, speed: f32) -> usize;
    fn reset(&mut self);
    fn set_speed(&mut self, speed: f32);
    fn is_exhausted(&self) -> bool;
    fn channels(&self) -> usize;
    fn sample_rate(&self) -> u32;
}
```

- `fill`：请求 `frames` 帧，内部按 `speed` 消耗输入，返回实际写入帧数。欠载补静音+计数。
- **关键设计**：变速在「源」里完成，写进环形缓冲的是已对齐主时钟速率的 PCM。

### `VarispeedSource`（`varispeed.rs`）

- 线性插值重采样：`phase += speed`，对每个采样做 `interp(phase)`。
- `interp(idx, ch)`：取 `floor(idx)` 和 `ceil(idx)` 两个样本线性插值；越界返回静音。
- `underrun_count`：`Arc<AtomicU64>` 跨线程共享，数据耗尽即 +1。
- **PCM 共享**：`samples` 是 `Arc<[f32]>`，`clone()` 只增加引用计数（不复制、不分配），
  所以实时线程可以为同一音效创建多个播放实例。
- `fill` 的 `speed` 是「每个输出帧前进的源帧数」。源与输出采样率不同时，由调用方乘上
  换算比（见 soundboard 的 `rate_ratio`）。
- `from_file(path)`：按扩展名选解码器（见 `audio_file.rs`）。
- `progress()` / `seek_ratio(r)`：读取 / 设置播放进度（0..1，按源帧计），只改相位，实时安全；
  用于界面进度条与拖动跳转。
- **语义**：第一阶段倍速 = varispeed（变速变调，像磁带）。保音高 stretch 留第二阶段。

### `WavFileSource`（`wav_file.rs`）

- 用 `hound` 读 WAV，统一转 f32 交错缓冲（i16/i32/u8/f32 都处理）。
- `DecodedAudio`：解码结果（samples + rate + channels + format）。

### `decode_audio_file`（`audio_file.rs`）

- WAV 走 `hound`；MP3/FLAC/OGG/AAC 等走 `symphonia`，统一输出 f32 交错 PCM。
- 会做文件 IO 与整段解码，**只能在控制侧调用**，不能进实时线程。

---

## 7. audio-engine：引擎三执行器与时钟

**位置**：`crates/audio-engine/src/`

### 三执行器（`executors.rs`）

引擎不硬编码线程，只定义**逻辑角色**：

| trait | 职责 | 实际跑在哪 |
|---|---|---|
| `ControlExecutor` | 处理命令、设备切换、参数验证 | 应用线程 |
| `DecodeExecutor` | 文件解码、变速、预填充环 | 独立后台线程 |
| `RealtimeCallback` | 消费命令(≤32)、取 PCM、混音/DSP、写出 | cpal 输出回调 |

`EngineCommand`：控制命令枚举（设增益、设速率、启停源、切设备、停机）。

### `DriftController`（`clock.rs`）

水位 PI 漂移控制：以环形缓冲**填充水位**为控制量，PI 输出限幅 ±1000 ppm，一阶低通平滑。

```rust
pub fn update(&mut self, current_fill: u32);  // 非实时线程周期调用
pub fn ratio(&self) -> f32;                    // 实时侧只读
```

- `ratio_bits`：f32 以 `AtomicU32` 位模式跨线程共享（不用非原子 f32）。
- **为什么不用「累计样本数估比率」？** f32 精度不足，且开环无反馈。水位 PI 是闭环。

### `Engine`（`engine.rs`）

引擎运行态：持有混音参数、线性增益、文件速率、运行标志。实时线程独占访问。
- `apply_command(cmd)`：实时侧应用命令。
- `mix(file, mic, out, frames, channels)`：两路相加 + `tanh` 软限幅。

---

## 8. application：装配层

**位置**：`crates/application/src/`

### `Mixer`（`mixer.rs`）

实时安全的混音器。增益/速率以 `Arc<AtomicU32>`（f32 位模式）跨线程共享。
- `mix(file, mic, out)`：`file*fg + mic*mg`，再 `tanh` 软限幅。
- `set_file_gain_db` / `set_mic_gain_db` / `set_file_speed`：控制侧调用，原子写入。

### `App`（`app.rs`）

装配核心：把后端、混音器、文件源接起来。

**`start()` 的线程拓扑**：
1. 打开输出流：实时回调读两个 SPSC 环 → 混音 → 写出。
2. 打开输入流：麦克风回调写麦克风环。
3. spawn 解码线程：文件源 `fill` → 声道转换 → 写文件环。

**`RunHandle`**：持有停止标志、解码线程、流句柄；drop 时自动 stop + join + 释放流。

**`convert_channels(src, src_ch, dst, dst_ch)`**：声道转换（mono↔stereo 等），保证文件与引擎声道数一致。

---

## 9. audio-cpal：硬件后端

**位置**：`adapters/audio-cpal/src/lib.rs`
**定位**：`AudioBackend` 的 cpal 实现，全平台通用（Windows WASAPI / macOS CoreAudio / Linux）。

### `CpalBackend`

- `new()`：`cpal::default_host()` 创建默认主机。
- `enumerate_devices()`：枚举设备并收集支持的输入/输出配置。
- `open_output(config, callback)` / `open_input(...)`：构建流、`play()` 启动。

**关键细节**：
- `pick_rate`：请求采样率不在设备范围时回退到最高支持值。
- `to_buffer_size`：`0` → 设备默认缓冲，否则固定帧数。
- 回调里直接调用户 callback，无额外拷贝。

### 按设备原生配置开流（音效板主程序使用）

```rust
let out = backend.resolve_output(Some("CABLE Input"))?; // 或 None = 默认设备
let cfg = out.config();          // 设备原生采样率 / 声道数
let handle = backend.build_output(out, 512, callback)?;
```

- `resolve_output(name)` / `resolve_input(name)`：按名称「包含匹配」（不区分大小写）找设备，
  `None` 用系统默认；返回 `ResolvedDevice`（设备 + 名称 + 原生配置）。
- **为什么先解析再开流**：WASAPI 共享模式只接受设备的混音格式。先拿到真实采样率，调用方
  才能据此准备回调状态（音效板的采样率换算、麦克风重采样）。
- `build_output(dev, buffer_frames, cb)`：`buffer_frames` 只是提示，回调实际收到的帧数可能
  不同，回调必须能处理任意长度。
- `build_input(dev, cb, on_error)`：`on_error` 在流出错（如蓝牙耳机断开）时调用，
  麦克风线程据此发现断开并重连。

---

## 10. desktop：CLI 壳

**位置**：`apps/desktop/src/main.rs`
**定位**：无 GUI 的命令行演示，验证端到端链路。

```text
soundbridge --list                          列出设备
soundbridge <file.wav> --speed 1.0 --file-gain 0 --mic-gain 0   播放文件+麦克风混音
```

- `main()`：解析参数 → `App::new` → `load_file` → 设增益/速率 → `start` → 等 Ctrl-C → `stop`。
- `ctrlc`：捕获 Ctrl-C 优雅停机。

---

## 11. soundboard：音效板核心

**位置**：`crates/soundboard/src/lib.rs`
**定位**：管理音效定义与播放实例，在输出回调里混音。整体设计见
[音效板架构](soundboard-architecture.md)。

### `LoadedSound`

```rust
let loaded = LoadedSound::load(spec)?;          // 控制侧：读文件 + 解码
cmd_tx.send(BoardCommand::AddSound(Box::new(loaded)))?;
```

- 把「读文件 + 解码」放在控制侧完成（服务端由加载线程负责），实时线程收到的已经是解码好的 PCM。
- 加载失败（文件不存在、格式不支持）在这里就返回错误，API 可以直接告诉用户。
- `decoded_bytes()`：解码后占用的内存，加载线程据此控制预加载总量。
- `LoadedSound` 可以 `Clone`：只复制 `Arc` 共享的 PCM，切换输出设备时据此把已解码的音效重新装进新的音效板；
  `with_spec(spec)` 换上最新的设置。

### `PlayMode`

- `{ looping, exclusive }` 两个独立的布尔开关，取代旧的四选一 `TriggerMode`。
- `From<TriggerMode>`：读旧数据时转换（`Loop` → 循环、`Exclusive` → 独占、其它 → 都关）。
- 触发逻辑：循环且已在播放 → 停止（再按一下停）；独占 → 先清空其它实例；然后新建实例。

### `SoundBoard`

- `new(capacity_samples, output_rate)`：预分配 scratch、实例表（`MAX_INSTANCES`）和定义表。
- `apply_command(cmd)`：在输出回调里执行命令。`Trigger` 只克隆共享 PCM，不分配；
  `SetMode` 更新定义并就地改正在播放实例的循环标志（不打断播放）；`Seek` 设置该音效所有实例的进度。
- `for_each_playing(|id, progress|)`：按启动先后回调每个播放实例，供 `PlayingSet` 发布给界面。
- `fill(out, frames, channels)`：覆盖写 `out`。每个实例按 `speed × rate_ratio` 步进；
  scratch 不够时分块。
- `fill_instance` / `accumulate`：关联函数（不借 `self`），让调用方可以同时借用
  `instances` 和 `scratch` 两个字段。

---

## 12. server：音效板服务库

**位置**：`apps/server/src/`（库 `soundbridge_server` + 浏览器版入口 `soundbridge-server`）

| 文件 | 内容 |
|---|---|
| `lib.rs` | `ServerConfig`（含宿主钩子 `Hooks`）、启动流程 `run()`、`Control`（关闭询问）、内嵌前端 `static_handler`、`lan_ip` |
| `main.rs` | 浏览器版入口：`ServerConfig::from_env()` → `run()` |
| `api.rs` | `AppState` + 全部 HTTP 处理函数；`apply_diff` 把扫描结果同步到加载与快捷键 |
| `library.rs` | `Library`：音效库 = 磁盘文件夹（扫描、文件夹操作、`.soundbridge.json`、旧版迁移） |
| `loader.rs` | `Player`：加载线程（预加载 + 按需加载）与唯一触发入口 |
| `audio.rs` | `start_audio`：输出线程（打开 / 切换输出设备）、构建输出回调；`describe_output`、`MicReader`、`PlayingSet` |
| `mic.rs` | `MicControl`（原子量参数 + 状态）、麦克风线程、`auto_pick`、`MicCapture`、`input_risk` |
| `monitor.rs` | 试听：`MonitorControl`、试听线程、`MonitorTap`（主回调侧）、`MonitorReader`（试听回调侧） |
| `remote.rs` | `RemoteServer`：手机遥控监听的启动 / 停止、访问码中间件 `require_token`、`qr_svg` |
| `settings.rs` | `SettingsStore`：读写 `settings.json`；`data_dir()`（按平台）；`CloseAction` |
| `i18n.rs` | 界面语言：`apply` / `is_en`，宏 `tr!` / `trf!` 按语言生成提示与错误 |
| `hotkeys.rs` | 全局热键线程（Win32 消息循环 + 绑定请求），按下时调用 `Player::trigger` |
| `virtual_mic.rs` | Linux 虚拟麦克风：`pactl` 创建 / 接管 / 卸载模块 |

### 启动顺序（`run`）

1. 读设置（`SettingsStore`）并应用界面语言（`i18n::apply`），决定输出设备：`ServerConfig.output_device` > 上次选择 > 系统默认。
2. Linux 且未指定输出设备时，`VirtualMic::setup()` 创建虚拟麦克风并设置 `PULSE_SINK` / `PIPEWIRE_NODE`
   （必须在打开音频流之前）。
3. `migrate_v1` 迁移旧版数据，`Library::open(root)` 打开并扫描音效库。
4. `Player::new` 启动加载线程；建 `MicControl`、`MonitorControl`；
   初始麦克风：`ServerConfig.input_device` > 上次选择 > 自动。
5. `audio::start_audio(...)`：启动输出线程（打开输出设备、构建输出回调）→ `spawn_mic_thread` →
   `spawn_monitor_thread`。
6. 启动热键线程；为每个音效绑定快捷键并排队预加载（不阻塞启动）。
7. 主界面只监听 `127.0.0.1`，绑定端口（App 允许回退随机端口）。
8. 建 `RemoteServer`（访问码来自设置，没有就生成），端口 = 主端口 + 1；
   设置 `lan_remote` 或 `SOUNDBRIDGE_LAN=1` 时立即启动。
9. 组装 `AppState`（含 `hooks`、`close_prompt`），把路由交给 `RemoteServer`，调用 `on_ready` 通知宿主，开始服务。

**输出线程**：cpal 的流句柄在部分平台不能跨线程，所以主输出流由专用线程 `soundbridge-output` 持有，
切换输出时也在这个线程里关旧流、开新流（`OutputRequest::Select { device, reply }`）。
宿主仍在专用线程上 `block_on(run(...))`。

### `RemoteServer`（`remote.rs`）

- `start()`：绑定 `0.0.0.0:preferred_port`（被占用则随机端口），用 `with_graceful_shutdown` 跑同一个 `Router`；
  `stop()`：发关闭信号，端口随即释放。开关都不用重启。
- `require_token`：查询参数 `t` 或 Cookie `sb_token` 必须等于访问码（带 `t` 时写 Cookie）；
  `DESKTOP_ONLY` 列表里的接口对手机只允许 GET。
- `status()`：`{enabled, url, port, error}`，`url` = `http://<局域网 IP>:<端口>/?t=<访问码>`。

### 宿主钩子与关闭询问（`lib.rs`）

- `Hooks { on_settings, on_window_action }`：服务库不依赖 Tauri，桌面 App 通过钩子接收「设置变了」
  （用来给图标着色）和「关闭询问的回答」（隐藏到托盘 / 退出）。
- `Control::request_close()`：置位 `close_prompt` 并返回之前是否已置位——已经在问了还再点 ×，
  说明界面没响应，外壳直接退出。界面经 `/api/live` 看到标记后弹框，`POST /api/window` 回答并清除标记。

### `Library`（`library.rs`）

- `open(root)`：建目录、读 `.soundbridge.json`、`scan`。
- `scan`：递归收集文件夹与音频文件 → 为新文件分配 id → 去掉已不存在的条目 → 写回设置文件 →
  返回 `ScanDiff { added, removed }`。
- `create_folder` / `rename_folder` / `delete_folder`、`import` / `rename_sound` / `move_sound` / `delete_sound`：
  先改磁盘，再迁移设置键（`rekey_prefix` / `move_entry`），最后 `scan`。所有输入路径经 `clean_rel` 清理。
- `set_root(new, move_contents)`：可选把旧库内容移过去（`move_dir_contents`，跨盘时复制后删除）。
- `migrate_v1(data_dir, root)`：旧版 `library.json` → 新音效库，详见架构文档 §7.1。

### `Player`（`loader.rs`）

- `trigger(id)`：已加载则直接发 `Trigger`；否则把 `LoadAndTrigger` 放进优先队列。
- `preload(id)` / `install(loaded)` / `unload(id)` / `unload_all()` / `send(cmd)`。
- 加载线程 `Worker::run`：先取优先队列，空了再用 `select!` 同时等两个队列；预加载累计超过
  `PRELOAD_BUDGET_BYTES` 后跳过预加载任务。每次都从 `Library::spec` 取最新定义再解码。

- `reinstall_all()`：切换输出后把缓存里所有已解码音效重新装进新的音效板。

### `PlayingSet`（`audio.rs`）

- 输出回调每次 `fill` 后调用 `publish(&board)`，把播放实例的（音效 id, 进度）写进两组固定的原子槽位，id 为 0 表示空。
- `/api/live` 读取 `snapshot()`：每个音效一项，多个实例时取最新的进度。无锁、不分配，实时安全。

### 热键线程（`hotkeys.rs`）

- `Bindings::bind`：解析 → 检查是否被其它音效占用 → 注册新键 → 注销旧键 → 更新表。
- `pump_messages`：Windows 上分发 `WM_HOTKEY`，没有它热键不会触发；其它平台为空函数。

### `MicControl`（`mic.rs`）

- `gain_db` / `muted` / `level`：原子量，实时回调直接读写。
- `status: Mutex<MicStatus>`：设备状态（选择、在用设备、是否连接、提示），**只在控制侧**加锁。
- `update_level(peak)`：快升慢降（每次回调衰减到 0.9 倍），给 UI 电平表用。

### 麦克风线程（`spawn_mic_thread`）

```
loop {
    recv_timeout(1s) {
        Select{device}  → open(device)：成功才替换旧流 + 记住选择；结果回给 API
        OutputChanged   → 按新输出的采样率重开、重新做反馈环检查
        超时            → watch()：断开则重连；自动模式下检测默认设备变化
    }
    释放输出回调送回的旧环
}
```

- `open(device)`：解析设备 → `input_risk` 检查 → 新建环 + `MicCapture` → `build_input` →
  把消费端经 `cons_tx` 交给输出回调 → 替换旧流。任一步失败都保留旧流。
- `set_unavailable(note)`：同一原因只记一次日志，避免每秒刷屏。
- `auto_pick()`：选择为「自动」时，系统默认录音设备是硬件麦克风就用它，否则取第一个硬件麦克风。

### `input_risk(input, output)`（输入设备检查）

- 返回 `Some(InputRisk::FeedbackLoop)`：输入是输出所在虚拟设备的另一端，输出会被录回自身。
- 返回 `Some(InputRisk::SystemLoopback)`：输入是立体声混音这类回录设备，会录进游戏声音。
- **为什么只看名称**：各平台没有统一 API 能查出「这个录音端点对应哪个播放端点」，
  而虚拟音频线的命名很规律（「端点 (驱动)」、两端同名等）。
- **为什么要求「是虚拟设备」**：同一块声卡的麦克风和扬声器驱动名也相同（如
  `Realtek(R) Audio`），蓝牙耳机的麦克风与听筒甚至完全同名（如 `耳机 (AirPods)`），
  但它们都不是反馈环，所以只有命中虚拟关键词才判反馈环。
- 规则和关键词表见 `mic.rs` 顶部常量与 [音效板架构 §6.2](soundboard-architecture.md)。

### `MicCapture`（输入回调侧）与 `MicReader`（输出回调侧）

- `MicCapture::push`：下混单声道 → 线性插值重采样（`step = 输入采样率 / 输出采样率`）→ 写环。
- `MicReader::mix_into`：先收新环（换设备）→ 漂移裁剪 → 抖动缓冲（primed）→ 按增益叠加到
  各声道 → 更新电平。

### 试听（`monitor.rs`）

- `MonitorTap::tap`（主输出回调，在叠加麦克风之前）：把只含音效的缓冲转成立体声写环；先收试听线程送来的新环，没有环（未开启或不需要试听）时直接返回。
- `MonitorReader::fill`（试听回调）：收新环 → 漂移裁剪（积压 > 60 ms 丢弃）→ 抖动缓冲（约 20 ms）→
  按 `step = 主输出采样率 / 试听采样率` 线性插值 → 乘试听音量。第一帧同时作为插值的前后两点，
  避免开头出现一个 0 样点。
- 试听线程：`Configure { enabled, device }` 开关 / 换设备；`MainChanged` 时主输出变了，重新判断
  是否需要试听（主输出是扬声器、或试听设备与主输出相同时不打开）。

---

## 13. app：桌面 App（Tauri 外壳）

**位置**：`apps/app/`

| 文件 | 内容 |
|---|---|
| `src/main.rs` | Tauri 入口：单实例插件、启动服务（传入钩子）、创建窗口、托盘、关闭询问、图标着色 |
| `tauri.conf.json` | 产品名、标识符、图标；窗口在代码里创建（`windows: []`） |
| `loading/index.html` | 服务启动失败时显示的错误页 |
| `icons/` | 应用图标（`icon.ico` 含 16/32/48/256 尺寸） |

- `start_server()`：开线程 → 建 tokio 运行时 → `block_on(run(config, on_ready))`，通过
  `mpsc` 通道把端口传回主线程，最多等 20 秒。
- `setup`：服务就绪则窗口加载 `WebviewUrl::External("http://127.0.0.1:<端口>")`，
  否则加载内置错误页。
- 窗口参数：1080×700、最小 760×520、背景色与界面一致（防白屏闪烁）、关闭 Tauri 拖放拦截。
- `build_tray`：托盘图标 + 菜单（打开 / 退出，按界面语言；`on_settings` 时 `apply_tray_menu` 重建），左键单击 `show_main`。
- `on_window_event(CloseRequested)`：按设置 `close_action` 隐藏 / 退出 / 询问（`Control::request_close`）。
- `tinted_icon(accent)`：内置 `icons/icon.png` 逐像素转 HSL，色相旋转「主题色色相 − 246.1°」，
  饱和度很低的像素（白色图案）不动；钩子在后台线程触发，图标在主线程（`run_on_main_thread`）设置。
- `#![windows_subsystem = "windows"]`：发布版不弹控制台。
- 构建：Windows 用 `build-app.ps1`（前端 → `cargo build --release -p soundbridge-app` → 复制 exe 与
  `WebView2Loader.dll` 到 `dist-app/SoundBridge/`）；Linux 用 `build-app.sh`（同样流程，产物为单个可执行文件）。

---

## 附录：命名约定

| 约定 | 说明 |
|---|---|
| `*_db` | dB 值（对数域） |
| `*_lin` | 线性值（幅度域） |
| `frames` | 帧数（一帧 = 每声道一个采样） |
| `samples` | 样点数（frames × channels） |
| `prod/cons` | ringbuf 的 producer / consumer |
| `ring` | SPSC 环形缓冲 |
