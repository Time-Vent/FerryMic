# FerryMic 移动端（占位）

> 本目录为 **Flutter 工程占位**，不参与 `cargo build --workspace`。

## 定位

移动端不提供系统级虚拟麦克风（见 `docs/adr/0004`）。定位为**独立产品形态**：

- 应用内音频处理、变声、录制
- 协作编辑端
- 输出经 **ffi** 暴露给宿主 App 或导出为文件，**不注入系统麦克风**

## 计划

- Flutter 工程经 ffi 生成 Rust 绑定，调用引擎（`audio-engine` / `application`）。
- 与其他 App 交换音频走 AUv3（iOS）或文件导出，不使用已弃用的 Inter-App Audio。

## 状态

第一阶段：仅占位，无实现。
