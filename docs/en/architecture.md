# FerryMic Architecture (phase-one implementation)

**English** | [简体中文](../architecture.md)

> This document describes the architecture **as implemented** (phase one: "core engine + desktop shell"). It follows
> design baseline v1.1 but records only what was delivered. Reserved items not yet implemented (loopback, virtual
> microphone, plugin isolation, collaboration) are marked "reserved".
>
> The architecture of the main product, the soundboard (desktop app `apps/app` + service library `apps/server` +
> `crates/soundboard`: mic + effects mixing, sound library, runtime mic switching, reconnect, feedback protection), is
> described in [soundboard-architecture.md](soundboard-architecture.md).
> The engine layer here (`audio-engine` / `application`) is currently used only by the `apps/desktop` CLI.

## 1. Purpose

FerryMic is a desktop real-time audio mixing tool: it captures the physical microphone and audio files, mixes them with
adjustable volume and speed, and outputs to the sound card after basic DSP. Phase one verifies the end-to-end path
"file (varispeed) + mic → independent gains → mix → sound card".

## 2. Design principles (in place)

1. **Contracts before implementations**: modules depend only on traits (`platform-api`) and plain data types
   (`audio-types`), not on a particular threading model, ring buffer library or platform API.
2. **Real-time callbacks never allocate, lock, do IO or panic**: the `audio-cpal` callback is the audio thread.
3. **DSP processors are stateful**: biquad delay lines and limiter envelopes persist across buffers.
4. **Fixed-size processors are separate from variable-rate sources**: varispeed changes the frame count, so it goes
   through `VariableRateSource`, not `FixedProcessor`.

## 3. Directory layout (implemented part)

```
FerryMic/
├── Cargo.toml            # workspace + shared dependency versions
├── rust-toolchain.toml   # pins stable + the GNU target
├── rustfmt.toml / clippy.toml / deny.toml
├── crates/
│   ├── audio-types/      # plain data types (no dependencies)
│   ├── platform-api/     # capability trait contracts
│   ├── dsp/              # FixedProcessor: gain / limiter / biquad
│   ├── audio-graph/      # sequential chain ProcessingGraph
│   ├── audio-source/     # VariableRateSource: WAV decoding + varispeed
│   ├── audio-engine/     # three executors + DriftController
│   └── application/      # App assembly + Mixer
├── adapters/
│   └── audio-cpal/       # AudioBackend implemented with cpal (all platforms)
└── apps/
    └── desktop/          # CLI demo shell
```

## 4. Dependency direction (one-way, acyclic)

```
apps/desktop
  └─ application ──┬─ audio-engine ──┬─ audio-graph ── dsp ── audio-types
                   │                 ├─ audio-source ── audio-types
                   │                 └─ platform-api ── audio-types
                   ├─ audio-cpal ────┴─ platform-api ── audio-types
                   └─ audio-source
```

Rule: **upper layers depend on lower layers, never the reverse**. `audio-types` is the only leaf with no internal dependencies.

## 5. Core interfaces

### 5.1 platform-api (capability traits)

- `AudioBackend`: enumerate devices + open input / output streams. Implemented by `audio-cpal`.
- `OutputStreamHandle` / `InputStreamHandle`: stream handles, stop on drop.
- `VirtualMic` / `SystemAudioLoopback` / `PermissionGate`: **reserved** (not implemented in phase one).

### 5.2 dsp (fixed-size processors)

- `FixedProcessor`: `process` (same frame count) + `reset` + `latency_samples` + `set_param`.
- Implementations: `Gain` (smoothed), `Limiter` (hard limit + release envelope), `Biquad` (RBJ cookbook, with delay lines).

### 5.3 audio-source (variable-rate sources)

- `VariableRateSource`: `fill` (consumes input according to speed; underruns are padded with silence and counted).
- Implementations: `VarispeedSource` (linear-interpolation resampling; speed and pitch change together), `WavFileSource` (decoded with hound).
- `InputSource`: abstraction for capture sources (reserved; phase one connects rings directly).

### 5.4 audio-engine (logical roles)

Three logical roles (no hard-coded threads):
- `ControlExecutor`: the app thread; commands, device switching, parameter validation.
- `DecodeExecutor`: its own thread; file decoding, varispeed, prefilling the file ring.
- `RealtimeCallback`: the output callback; drains commands (≤32), fetches PCM, mixes / DSP, writes out.

`DriftController`: fill-level PI drift control (±1000 ppm, low-pass smoothed), intended for asynchronous resampling in phase two.

## 6. Clock architecture

- **Master clock = the output device callback** (the cpal output callback).
- The file source flows in through the decode thread + an SPSC ring; the microphone through the input callback + an SPSC ring.
- In phase one the file source is varispeed (resampled directly by speed), so `DriftController`'s closed loop isn't
  enabled yet; the controller is implemented and available for clock-drift alignment in phase two.

## 7. Threads (phase one, actual)

| Thread | Job |
|---|---|
| cpal output callback | read the file ring + mic ring → mix → write the output (real-time path) |
| decode thread | file source fill → channel conversion → write the file ring |
| cpal input callback | mic data → write the mic ring |
| app main thread | CLI arguments, start, stop on Ctrl-C |

## 8. Platform capabilities (phase one)

| Platform | Implemented | Not implemented |
|---|---|---|
| Windows | file + mic mixed to the sound card | loopback, virtual audio cable |
| macOS | same (cpal is cross-platform) | loopback, virtual microphone |
| Linux | same | loopback, virtual microphone |
| Mobile | not applicable | everything |

## 9. Reserved items (later phases)

- **Loopback**: Windows device / process loopback (paths A / B), see baseline §7.
- **Virtual microphone**: macOS AudioServerPlugin / Linux PipeWire, see baseline §11.
- **Plugins**: plugin-api trait + C ABI + process isolation, see baseline §8.
- **Collaboration**: CRDT parameter snapshots, see baseline §10.
