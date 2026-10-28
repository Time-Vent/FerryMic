# ADR-0004：虚拟麦克风分平台实现路径

**状态**：预留 Proposed（第二阶段先实测）

**日期**：2026-10-27

## 背景

「把混音注入系统麦克风」是核心诉求，但各平台能力边界差异巨大。

## 决策

**Windows（第一阶段）**：不绑定 VB-Cable，不实现 VirtualMic。策略为「输出目标选择」：
枚举系统渲染端点，让用户选择声卡或已安装的虚拟音频线，经 `AudioBackend::open_output`
打开，其回调即主时钟。自研 WDM 驱动放后续阶段。

**macOS（第二阶段）**：AudioServerPlugin（HAL 用户态插件），**不是** DriverKit
（AudioDriverKit 仅面向物理设备）。进入前必须实测 tympan-aspl 能否加载。安装成本：
管理员权限写 HAL 目录、签名公证、重启 coreaudiod。无权限时 Core Audio 返回静音而非
报错，权限申请必须显式做。

**Linux（第二阶段）**：优先 PipeWire（`PW_KEY_MEDIA_CLASS = "Audio/Source"` 或
filter-chain），PulseAudio module-pipe-source 兜底。

**移动端**：不提供系统级虚拟麦克风。iOS 沙箱与音频进程模型不允许第三方应用创建全局
虚拟输入设备；Android 受 AudioFlinger 与厂商策略限制。移动端定位为独立产品形态
（应用内处理、变声、录制、协作编辑端），输出经 ffi 或文件导出，不注入系统麦克风。
删除 Inter-App Audio（iOS 13 弃用）。

## 后果

- Windows 是第一阶段唯一完整交付平台；macOS/Linux 第一阶段只验证引擎正确性。
