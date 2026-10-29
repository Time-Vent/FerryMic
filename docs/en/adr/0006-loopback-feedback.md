# ADR-0006: Loopback capture and feedback protection

**English** | [简体中文](../../adr/0006-loopback-feedback.md)

**Status**: Proposed (the main path for Windows in phase one)

**Date**: 2026-10-27

## Context

If Windows system audio capture mixes up "capture the whole system mix" and "capture a specific device", it leads to
feedback loops (the output captured as input again) and missing "devices must differ" checks.

## Decision

Two capture paths that **must not be mixed**:

**Path A: process loopback (no device selected)**
- Captures the whole system mix, including / excluding by process, always excluding our own PID.
- Probed at runtime via `ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`; falls back to path B if the probe fails.
- Officially needs build 20348 or later (Windows 11 / Server 2022); the version number isn't hard-coded.

**Path B: device loopback (device selected)**
- The user explicitly chooses the loopback source device; the "default render endpoint" is never assumed.
- A hard check before starting: `loopback_source != output_target`; if equal, return `LoopbackError::FeedbackLoopDetected`.
- Register an `IMMNotificationClient` to watch device changes; re-check after changes at runtime and pause on failure.

## Guards

- `ProcessLoopbackGuard`: checks only the runtime probe result and the own-process exclusion.
- `DeviceLoopbackGuard`: checks that the devices differ before starting + re-checks on device changes at runtime.

## Other platforms

- Linux: PipeWire monitor source; PulseAudio `.monitor` as a fallback (not implemented in phase one).
- macOS: ScreenCaptureKit (macOS 13+, needs screen recording permission; not implemented in phase one).

## Consequences

- Feedback detection covers both paths; tests live in `tests/audio-quality/feedback/`.

## Implemented: microphone input checks (soundboard)

The soundboard's mic capture has a feedback risk too: when outputting to a virtual cable, picking the other end of the
same cable as the mic records the output back into itself. `input_risk(input, output)` in `apps/server/src/mic.rs`
checks device names before opening the mic, **without depending on any vendor**:

- `FeedbackLoop`: same name as the output and virtual, or the same driver name in parentheses and virtual (VB-Cable,
  VoiceMeeter, VAC, BlackHole, …), or — without parentheses — the same name after removing direction words and virtual.
  A Bluetooth headset's mic and earpiece share a name but aren't virtual devices, so they aren't misjudged.
- `SystemLoopback`: loopback devices such as Stereo Mix / What U Hear / Linux `Monitor of …`, which would record
  everything the computer plays into the mic, are refused as well.

Matching devices are refused; "Auto" mic selection also skips them and any other virtual device, picking only hardware
mics. After an output switch, the mic thread re-checks against the new output. The same rule derives "which mic other
apps should pick" (`game_mic_for`: the input that forms a loop with the output is the other end of the cable).
This is a name heuristic, so unusually named cables may slip through; see the unit tests in `mic.rs`
such as `detects_feedback_loop_across_vendors` and `allows_real_mics_and_different_cables`.

**The monitor can't form a loop**: the monitor stream carries only the effects, never the mic, and isn't opened when
the main output is itself a speaker or when the monitor device equals the main output.
