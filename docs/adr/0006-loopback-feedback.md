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
