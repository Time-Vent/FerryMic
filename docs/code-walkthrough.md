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
- **语义**：第一阶段倍速 = varispeed（变速变调，像磁带）。保音高 stretch 留第二阶段。

### `WavFileSource`（`wav_file.rs`）

- 用 `hound` 读 WAV，统一转 f32 交错缓冲（i16/i32/u8/f32 都处理）。
- `DecodedAudio`：解码结果（samples + rate + channels + format）。

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

## 附录：命名约定

| 约定 | 说明 |
|---|---|
| `*_db` | dB 值（对数域） |
| `*_lin` | 线性值（幅度域） |
| `frames` | 帧数（一帧 = 每声道一个采样） |
| `samples` | 样点数（frames × channels） |
| `prod/cons` | ringbuf 的 producer / consumer |
| `ring` | SPSC 环形缓冲 |
