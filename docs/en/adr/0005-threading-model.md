# ADR-0005: Threading model (logical roles separate from physical threads)

**English** | [简体中文](../../adr/0005-threading-model.md)

**Status**: Accepted

**Date**: 2026-10-27

## Context

Real-time audio is extremely sensitive to thread boundaries. Conflating "logical roles" with "physical threads" would
make the engine impossible to port (audio callback threading differs between platforms).

## Decision

The engine doesn't hard-code a number of physical threads; it defines only three **logical execution roles**:

| Role | Job | Physical thread (phase one) |
|---|---|---|
| `ControlExecutor` | commands, device switching, parameter validation, plugin management | app thread |
| `DecodeExecutor` | file decoding, varispeed, prefilling SPSC rings | a background thread |
| `RealtimeCallback` | drains commands (≤32), fetches PCM, mixes / DSP, writes out | the cpal output callback |

Input devices (microphone, loopback) with their own callbacks feed the mix point through their own SPSC rings +
asynchronous resampling, without assuming they share the output's thread.

## Constraints

- Real-time callbacks never allocate, lock, do IO or panic.
- Cross-thread state is shared through lock-free primitives (`Atomic*`); f32 values use the `AtomicU32` bit pattern, never a non-atomic f32.

## Consequences

- The engine can be ported to any audio backend; the number of physical threads is decided by the platform adapter.
