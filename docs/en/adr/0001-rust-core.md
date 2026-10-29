# ADR-0001: Rust as the core implementation language

**English** | [简体中文](../../adr/0001-rust-core.md)

**Status**: Accepted

**Date**: 2026-10-27

## Context

Real-time audio mixing has strict requirements on latency, jitter and memory safety. A language has to be chosen for the engine core.

## Decision

Implement the core engine, DSP and platform adapters in **Rust**.

Reasons:
- No GC and no runtime pauses, which meets the "never allocate, never block" rule of real-time callbacks.
- The ownership model rules out data races at compile time, which suits multi-threaded audio (decode thread / callback thread).
- `cpal` provides a mature cross-platform audio backend (Windows WASAPI / macOS CoreAudio / Linux).
- A C ABI boundary can be exported to Flutter (a future mobile / UI layer) through ffi.

## Consequences

- Positive: real-time safety is enforced by the compiler; `#![forbid(unsafe_code)]` is applied in most crates.
- Negative: a steep learning curve; on Windows the GNU toolchain needs MinGW-w64 (or MSVC + VS Build Tools).
