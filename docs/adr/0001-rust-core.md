# ADR-0001：以 Rust 作为核心实现语言

**状态**：已接受 Accepted

**日期**：2026-10-27

## 背景

实时音频混音对延迟、抖动、内存安全有严格要求。需要选择一种语言作为引擎核心。

## 决策

以 **Rust** 实现核心引擎、DSP、平台适配器。

理由：
- 无 GC、无运行时停顿，满足实时回调「不分配、不阻塞」约束。
- 所有权模型在编译期保证无数据竞争，适合多线程音频（解码线程/回调线程）。
- `cpal` 提供跨平台音频后端（Windows WASAPI / macOS CoreAudio / Linux），成熟可用。
- C ABI 边界可导出给 Flutter（未来移动端/UI）经 ffi 调用。

## 后果

- 正向：实时安全由编译器约束，`#![forbid(unsafe_code)]` 在大多数 crate 强制。
- 负向：学习曲线陡；Windows 下 GNU 工具链需 MinGW-w64（或 MSVC + VS Build Tools）。
