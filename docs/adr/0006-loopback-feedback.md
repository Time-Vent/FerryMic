# ADR-0006：Loopback 采集与反馈环防护

**状态**：预留 Proposed（Windows 第一阶段主力路径）

**日期**：2026-10-27

## 背景

Windows 系统声音采集若混写「抓整个系统混音」与「抓指定设备」，会导致反馈环
（把输出又当作输入）与设备不等校验缺失。

## 决策

两条采集路径，**不得混写**：

**路径 A：Process Loopback（不选设备）**
- 抓取整个系统混音，按进程包含/排除，排除自身进程 PID。
- 经 `ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`
  运行时探测；探测失败自动降级到路径 B。
- 官方最低 Build 20348（Windows 11 / Server 2022），不写死版本号。

**路径 B：设备 Loopback（选设备）**
- 用户显式选择 loopback 源设备，不假设「默认渲染端点」。
- 启动前硬校验 `loopback_source != output_target`，相同则返回
  `LoopbackError::FeedbackLoopDetected`。
- 注册 `IMMNotificationClient`，监听设备变更；运行中变更后重新校验，失败则暂停。

## 防护结构

- `ProcessLoopbackGuard`：仅检查运行时探测结果与自身进程排除配置。
- `DeviceLoopbackGuard`：启动前校验设备不等 + 运行中设备变更重校验。

## 其他平台

- Linux：PipeWire monitor source；PulseAudio `.monitor` 兜底（第一阶段不实现）。
- macOS：ScreenCaptureKit（macOS 13+，需屏幕录制权限，第一阶段不实现）。

## 后果

- 反馈环检测覆盖两条路径，测试在 `tests/audio-quality/feedback/`。

## 已实现：麦克风输入检查（音效板）

音效板的麦克风采集也存在反馈环风险：输出到虚拟音频线时，若麦克风选了同一条线的另一端，
输出会被录回自身。`apps/server/src/mic.rs` 的 `input_risk(input, output)` 在打开麦克风前
按设备名检查，**不绑定任何厂商**：

- `FeedbackLoop`：与输出同名且是虚拟设备，或括号内驱动名相同且是虚拟设备（VB-Cable、VoiceMeeter、
  VAC、BlackHole 等），或无括号时去掉方向词后同名且是虚拟设备。蓝牙耳机的麦克风与听筒同名，
  但不是虚拟设备，因此不会被误判。
- `SystemLoopback`：立体声混音 / What U Hear / Linux `Monitor of …` 等回录设备，会把游戏等
  所有正在播放的声音录进麦克风，同样拒绝。

命中时拒绝打开；「自动」选择麦克风时也会跳过这些设备和其它虚拟设备，只选硬件麦克风。
切换输出设备后，麦克风线程会按新输出重新检查。同一条规则还用来推算「游戏里该选哪个麦克风」
（`game_mic_for`：与输出构成反馈环的那个输入就是虚拟线的另一端）。
这是名称启发式，命名特殊的虚拟线可能漏判；单元测试见 `mic.rs` 的
`detects_feedback_loop_across_vendors`、`allows_real_mics_and_different_cables` 等。

**试听不构成反馈环**：试听流只放音效、不放麦克风；主输出本身是扬声器、或试听设备与主输出相同时不打开。
