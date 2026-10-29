# ADR-0002: Plugin process isolation

**English** | [简体中文](../../adr/0002-plugin-isolation.md)

**Status**: Proposed (to be implemented in phase two)

**Date**: 2026-10-27

## Context

Third-party plugins may crash, loop forever or access things maliciously; they must not be able to affect the host engine process.

## Decision

Plugins are **process-isolated** by default (phase two):

- A C ABI solves the cross-language boundary but **not crash isolation** — a crash inside a dylib takes the host down with it.
- Desktop: process isolation + IPC latency compensation + timeout reclamation.
- WASM as an optional sandbox path (one viable path on mobile, but not the only one).
- Mobile plugins use the platform's proper mechanism (iOS AUv3, where the system enforces a separate process) or built-in DSP.

## Constraints

- Rust traits don't cross dynamic library boundaries; the C ABI must be versioned.
- Introducing process isolation requires introducing IPC latency compensation and timeout reclamation at the same time.

## Consequences

- Phase one only has first-party in-process plugins (gain / limiter examples); isolation is left for phase two.
