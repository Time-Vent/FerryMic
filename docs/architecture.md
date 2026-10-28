# FerryMic 架构文档（第一阶段实现版）

> 本文档描述**已实现**的架构（第一阶段「核心引擎 + 桌面壳」），与设计基线 v1.1 保持一致，
> 但只记录实际交付的内容。未实现的预留项（loopback、虚拟麦克风、插件隔离、协作）明确标注为「预留」。
>
> 主产品「音效板」（桌面 App `apps/app` + 服务库 `apps/server` + `crates/soundboard`：麦克风 +
> 音效混合、音效库、运行中切换麦克风、断开重连、反馈环防护）的架构见
> [soundboard-architecture.md](soundboard-architecture.md)。
> 本文的引擎层（`audio-engine` / `application`）目前只被 `apps/desktop` CLI 使用。

## 1. 定位

FerryMic 是一个桌面端实时音频混音工具：采集物理麦克风、音频文件，按可调音量与倍速混合，
经基础 DSP 处理后输出到声卡。第一阶段验证「文件(变速) + 麦克风 → 独立增益 → 混音 → 声卡输出」
端到端链路。

## 2. 设计原则（已落地）

1. **契约先于实现**：模块只依赖 trait（`platform-api`）与纯数据类型（`audio-types`），
   不依赖具体线程模型、具体环形缓冲库、具体平台 API。
2. **实时回调不分配、不加锁、不做 IO、不 panic**：`audio-cpal` 的回调即音频线程。
3. **DSP 处理器有状态**：biquad 延迟线、limiter 包络跨 buffer 保存。
4. **定长处理器与变速源分离**：变速改变帧数，不走 `FixedProcessor`，走 `VariableRateSource`。

## 3. 目录结构（已实现部分）

```
soundbridge/
├── Cargo.toml            # workspace + 统一依赖版本
├── rust-toolchain.toml   # 锁定 stable + GNU 目标
├── rustfmt.toml / clippy.toml / deny.toml
├── crates/
│   ├── audio-types/      # 纯数据类型（无依赖）
│   ├── platform-api/     # 能力 trait 契约
│   ├── dsp/              # FixedProcessor: gain/limiter/biquad
│   ├── audio-graph/      # 顺序处理链 ProcessingGraph
│   ├── audio-source/     # VariableRateSource: WAV 解码 + varispeed
│   ├── audio-engine/     # 三执行器 + DriftController
│   └── application/      # App 装配 + Mixer
├── adapters/
│   └── audio-cpal/       # cpal 实现 AudioBackend（全平台）
└── apps/
    └── desktop/          # CLI 演示壳
```

## 4. 模块依赖方向（单向，无环）

```
apps/desktop
  └─ application ──┬─ audio-engine ──┬─ audio-graph ── dsp ── audio-types
                   │                 ├─ audio-source ── audio-types
                   │                 └─ platform-api ── audio-types
                   ├─ audio-cpal ────┴─ platform-api ── audio-types
                   └─ audio-source
```

依赖规则：**上层依赖下层，下层绝不依赖上层**。`audio-types` 是唯一无内部依赖的叶子。

## 5. 核心接口

### 5.1 platform-api（能力 trait）

- `AudioBackend`：枚举设备 + 打开输入/输出流。由 `audio-cpal` 实现。
- `OutputStreamHandle` / `InputStreamHandle`：流句柄，drop 停止。
- `VirtualMic` / `SystemAudioLoopback` / `PermissionGate`：**预留**（第一阶段不实现）。

### 5.2 dsp（定长处理器）

- `FixedProcessor`：`process`（同帧数）+ `reset` + `latency_samples` + `set_param`。
- 实现：`Gain`（带平滑）、`Limiter`（硬限幅+释放包络）、`Biquad`（RBJ cookbook，有延迟线）。

### 5.3 audio-source（变速源）

- `VariableRateSource`：`fill`（按 speed 消耗输入，欠载补静音+计数）。
- 实现：`VarispeedSource`（线性插值重采样，变速变调）、`WavFileSource`（hound 解码）。
- `InputSource`：输入采集源抽象（预留，第一阶段用环直接连）。

### 5.4 audio-engine（逻辑职责）

三个逻辑角色（不硬编码线程）：
- `ControlExecutor`：应用线程，处理命令、设备切换、参数验证。
- `DecodeExecutor`：独立线程，文件解码、变速、预填充文件环。
- `RealtimeCallback`：输出回调，消费命令(≤32)、取 PCM、混音/DSP、写出。

`DriftController`：水位 PI 漂移控制（±1000 ppm，低通平滑），供第二阶段异步重采样用。

## 6. 时钟架构

- **主时钟 = 输出设备回调**（cpal 输出回调）。
- 文件源经解码线程 + SPSC 环汇入；麦克风经输入回调 + SPSC 环汇入。
- 第一阶段文件源为 varispeed（直接按速度重采样），暂未启用 `DriftController` 的闭环；
  该控制器已实现并可用于第二阶段的时钟漂移对齐。

## 7. 线程拓扑（第一阶段实际）

| 线程 | 职责 |
|---|---|
| cpal 输出回调线程 | 读文件环+麦克风环 → 混音 → 写输出（实时路径） |
| 解码线程 | 文件源 fill → 声道转换 → 写文件环 |
| cpal 输入回调线程 | 麦克风数据 → 写麦克风环 |
| 应用主线程 | CLI 参数、启动、Ctrl-C 停止 |

## 8. 平台能力（第一阶段）

| 平台 | 已实现 | 未实现 |
|---|---|---|
| Windows | 文件+麦克风混音输出到声卡 | loopback、虚拟音频线 |
| macOS | 同上（cpal 跨平台） | loopback、虚拟麦克风 |
| Linux | 同上 | loopback、虚拟麦克风 |
| 移动端 | 不适用 | 全部 |

## 9. 预留项（后续阶段）

- **Loopback**：Windows 设备/进程 loopback（路径 A/B），见基线 §7。
- **虚拟麦克风**：macOS AudioServerPlugin / Linux PipeWire，见基线 §11。
- **插件**：plugin-api trait + C ABI + 进程隔离，见基线 §8。
- **协作**：CRDT 参数快照，见基线 §10。
