# ADR-0004: Per-platform paths to a virtual microphone

**English** | [简体中文](../../adr/0004-virtual-mic-per-platform.md)

**Status**: Proposed (to be tested first in phase two)

**Date**: 2026-10-27

## Context

"Injecting the mix into the system microphone" is the core requirement, but platform capabilities differ enormously.

## Decision

**Windows (phase one)**: no dependency on VB-Cable and no `VirtualMic` implementation. The strategy is "choose the
output target": enumerate the system's render endpoints, let the user pick a sound card or an installed virtual audio
cable, and open it via `AudioBackend::open_output`, whose callback is the master clock. A custom WDM driver is left for later.

> **Implemented**: Settings → "Audio chain" → "Output line" switches the output target at runtime (`POST /api/output`,
> no restart, the choice is remembered). Outputs are grouped by name into virtual cables / speakers, and the mic other
> apps should pick (the other end of the cable, e.g. `CABLE Input` → `CABLE Output`) is derived and shown in the bottom
> bar (as "Mic to pick in other apps"). With the output on a virtual cable you can't hear the effects yourself, so there's
> also a "Monitor": an effects-only mix played on headphones / speakers at the same time (a second output stream).

**macOS (phase two)**: an AudioServerPlugin (HAL user-space plugin), **not** DriverKit (AudioDriverKit only targets
physical devices). Before starting, test whether tympan-aspl can be loaded. Installation cost: admin rights to write
the HAL directory, signing and notarisation, restarting coreaudiod. Without permission Core Audio returns silence
rather than an error, so permission must be requested explicitly.

**Linux (phase two)**: prefer PipeWire (`PW_KEY_MEDIA_CLASS = "Audio/Source"` or filter-chain), with PulseAudio
module-pipe-source as a fallback.

> **Implemented (simplified path)**: `apps/server/src/virtual_mic.rs` loads `module-null-sink` (`soundbridge_mix`) +
> `module-remap-source` (`FerryMic-Microphone`) with `pactl`, which works with both PipeWire (pipewire-pulse) and
> PulseAudio; this process routes its playback into the virtual output through the `PULSE_SINK` / `PIPEWIRE_NODE`
> environment variables, unloads the modules on exit, and takes over leftover modules on the next start. It compiles
> and passes tests in CI (Ubuntu 22.04) but hasn't been tried on a real Linux desktop yet. Creating an `Audio/Source`
> node with the native PipeWire API remains a possible later improvement.

**Mobile**: no system-level virtual microphone. The iOS sandbox and audio process model don't let third-party apps
create a global virtual input device; Android is limited by AudioFlinger and vendor policies. Mobile is positioned as a
separate product (in-app processing, voice changing, recording, a collaborative editor), with output exposed via ffi or
exported as files, never injected into the system microphone. Inter-App Audio (deprecated in iOS 13) is dropped.

## Consequences

- Windows is the only platform fully delivered in phase one; macOS / Linux only verify engine correctness in phase one.
