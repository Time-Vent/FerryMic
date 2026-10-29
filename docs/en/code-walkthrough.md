# FerryMic Code Walkthrough (for learning)

**English** | [简体中文](../code-walkthrough.md)

> This document explains each crate, module, struct, trait and key function, to make the code easier
> to learn. Each entry says where it is, what it is, why it's designed that way, and how its key methods are used.
>
> New to the project? Read the [Learning guide](learning-guide.md) first — it tells you where to start and in what order to read.

## Contents

1. [Overview](#1-overview)
2. [audio-types: plain data types](#2-audio-types-plain-data-types)
3. [platform-api: capability contracts](#3-platform-api-capability-contracts)
4. [dsp: fixed-size processors](#4-dsp-fixed-size-processors)
5. [audio-graph: processing chain](#5-audio-graph-processing-chain)
6. [audio-source: sources and varispeed](#6-audio-source-sources-and-varispeed)
7. [audio-engine: three executors and the clock](#7-audio-engine-three-executors-and-the-clock)
8. [application: assembly layer](#8-application-assembly-layer)
9. [audio-cpal: hardware backend](#9-audio-cpal-hardware-backend)
10. [desktop: CLI shell](#10-desktop-cli-shell)
11. [soundboard: soundboard core](#11-soundboard-soundboard-core)
12. [server: soundboard service library](#12-server-soundboard-service-library)
13. [app: desktop app (Tauri shell)](#13-app-desktop-app-tauri-shell)

---

## 1. Overview

The project is a Cargo workspace built on **layers with one-way dependencies**:

```
apps/desktop (CLI shell)
   └─ application (assembly: wires the layers together)
        ├─ audio-engine (logical executors + clock)
        │    ├─ audio-graph (processing chain)
        │    │    └─ dsp (gain / limiter / EQ)
        │    ├─ audio-source (files / varispeed)
        │    └─ platform-api (capability traits)
        ├─ audio-cpal (cpal hardware implementation)
        └─ audio-source
             └─ audio-types (plain data, the bottom layer)
```

**Why layers?** Real-time audio is extremely sensitive to blocking and allocation. With layers, the real-time
path only touches plain data and preallocated buffers at the bottom, while business logic (device enumeration,
file decoding) is isolated on its own threads. Contracts (traits) are separate from implementations (cpal), so
swapping the hardware backend or adding a platform doesn't touch the engine.

---

## 2. audio-types: plain data types

**Location**: `crates/audio-types/src/lib.rs`
**Role**: the only "leaf" crate with no internal dependencies. Modules pass these plain values to each other — never handles or locks.

### Structs / enums

| Type | Meaning |
|---|---|
| `SampleRate(u32)` | Sample rate in Hz, e.g. `48000` |
| `ChannelCount(u16)` | Channel count, 1 = mono, 2 = stereo |
| `FrameCount(usize)` | Frame count (one frame = one sample per channel) |
| `StreamConfig` | Rate + channels + buffer frames: the core parameters for opening a stream |
| `SampleFormat` | Sample format (F32 / I16); the engine uses F32 internally |
| `DeviceId(String)` | Opaque unique device id |
| `DeviceInfo` | Device description (name, default or not, supported input / output configs) |
| `EngineEvent` | Engine events (underrun, drift warning, device removed, …) |
| `Gain(f32)` / `PlaySpeed(f32)` | Gain / speed wrappers with clamping semantics |

### Key methods

- `Gain::clamped()`: clamps the gain to `0..64`, NaN becomes zero — keeps bad parameters out of the real-time path.
- `PlaySpeed::clamped()`: clamps the speed to `0.0625..16`; invalid values fall back to 1.0.

**Design point**: `StreamConfig` and `Gain` are `Copy`, so passing them between threads costs nothing and never allocates.

---

## 3. platform-api: capability contracts

**Location**: `crates/platform-api/src/lib.rs`
**Role**: defines only traits and error types, no platform code — "contracts before implementations" in practice.

### Traits

| Trait | Job |
|---|---|
| `AudioBackend` | enumerate devices + open input / output streams (the core) |
| `OutputStreamHandle` | output stream handle, `pause` / `resume`, stops on drop |
| `InputStreamHandle` | input stream handle |
| `VirtualMic` | virtual microphone (**reserved**) |
| `SystemAudioLoopback` | system audio loopback (**reserved**) |
| `PermissionGate` | permission gate (**reserved**) |

### Key signature

```rust
fn open_output<F>(&self, config: &StreamConfig, callback: F)
    -> Result<Box<dyn OutputStreamHandle>, PlatformError>
where F: FnMut(&mut [f32]) + Send + 'static;
```

- `callback` is the real-time callback, called by the backend on the **audio thread**. `Send + 'static` means it may move across threads and lives indefinitely.
- It returns `Box<dyn OutputStreamHandle>`: a trait object decouples callers from the concrete backend type.

### `PlatformError`

Derived with `thiserror`; covers missing devices, unsupported configs, permissions, loopback feedback detection and
more. Note: the real-time path **must not** construct it (it carries heap-allocated Strings).

---

## 4. dsp: fixed-size processors

**Location**: `crates/dsp/src/`
**Role**: every processor whose output frame count equals its input. Varispeed (which changes the frame count) lives in audio-source instead.

### `FixedProcessor` trait (`fixed.rs`)

```rust
pub trait FixedProcessor: Send {
    fn process(&mut self, input: &[f32], output: &mut [f32], frames: usize);
    fn reset(&mut self);
    fn latency_samples(&self) -> usize { 0 }
    fn set_param(&mut self, id: u32, value: f32);
    fn name(&self) -> &'static str;
}
```

- `process`: same-length processing; input and output are interleaved samples (frames × channels).
- `reset`: clears delay lines / envelopes (on device switches or seeks).
- `latency_samples`: fixed latency introduced by the processor (0 by default).
- `set_param`: parameter updates; `id` uses the constants in the `param` module.

### Implementations

**`Gain` (`gain.rs`)**: gain with one-pole smoothing to remove zipper noise.
- `set_gain_db(db, smoothing_samples)`: `smoothing_samples = 0` applies at once, otherwise it glides.
- Key point: `advance()` approaches the target every sample, avoiding clicks from gain jumps.

**`Limiter` (`limiter.rs`)**: peak limiter against clipping.
- Hard limit + exponential release envelope: anything over the threshold is pulled down at once, then recovers towards 1 by `release_coeff`.
- `envelope` is state carried across buffers; `reset` sets it back to 1.

**`Biquad` (`biquad.rs`)**: biquad EQ with RBJ cookbook coefficients.
- `BiquadParams::rbj(kind, fs, freq, q, gain_db)`: produces normalised coefficients.
- The delay lines `x1/x2/y1/y2` are kept per channel across buffers — a typical stateful processor.

**`db_to_linear` / `linear_to_db`**: dB ↔ linear amplitude conversion, shared by the whole project.

---

## 5. audio-graph: processing chain

**Location**: `crates/audio-graph/src/lib.rs`
**Role**: chains several `FixedProcessor`s in order (the simplest form for phase one; a DAG in phase two).

### `ProcessingGraph`

- `new(capacity_samples)`: preallocates two scratch buffers (double buffering).
- `push_processor(p)`: appends a node (before the real-time path starts).
- `process(input, output, frames, channels)`: the core — double-buffer ping-pong.

**Double buffering**: node 0 reads `scratch_a` and writes `scratch_b`, node 1 reads `scratch_b` and writes
`scratch_a`, alternating so no node allocates a new buffer. The result ends up in whichever buffer the last node wrote.

**Why doesn't `process` allocate?** Both buffers are allocated once in `new`; afterwards only slices are borrowed, which is real-time safe.

---

## 6. audio-source: sources and varispeed

**Location**: `crates/audio-source/src/`

### `VariableRateSource` trait (`lib.rs`)

```rust
pub trait VariableRateSource: Send {
    fn fill(&mut self, out: &mut [f32], frames: usize, speed: f32) -> usize;
    fn reset(&mut self);
    fn set_speed(&mut self, speed: f32);
    fn is_exhausted(&self) -> bool;
    fn channels(&self) -> usize;
    fn sample_rate(&self) -> u32;
}
```

- `fill`: requests `frames` frames, consuming input according to `speed`, and returns the frames actually written. Underruns are padded with silence and counted.
- **Key design**: speed change happens inside the source, so the PCM written into ring buffers is already at the master clock's rate.

### `VarispeedSource` (`varispeed.rs`)

- Linear-interpolation resampling: `phase += speed`, `interp(phase)` per sample.
- `interp(idx, ch)`: interpolates between the samples at `floor(idx)` and `ceil(idx)`; out of range returns silence.
- `underrun_count`: an `Arc<AtomicU64>` shared across threads, incremented when data runs out.
- **Shared PCM**: `samples` is an `Arc<[f32]>`; `clone()` only bumps the reference count (no copy, no allocation),
  so the real-time thread can create several playing instances of the same sound.
- `speed` in `fill` is "source frames advanced per output frame". When source and output rates differ, the caller
  multiplies in the rate ratio (see `rate_ratio` in soundboard).
- `from_file(path)`: picks a decoder by extension (see `audio_file.rs`).
- `progress()` / `seek_ratio(r)`: read / set the playback position (0..1, in source frames); only changes the phase,
  real-time safe; used for the UI's seek bar.
- **Semantics**: in phase one, speed = varispeed (speed and pitch change together, like tape). Pitch-preserving stretching is left for phase two.

### `WavFileSource` (`wav_file.rs`)

- Reads WAV with `hound` into an interleaved f32 buffer (handles i16 / i32 / u8 / f32).
- `DecodedAudio`: the decoding result (samples + rate + channels + format).

### `decode_audio_file` (`audio_file.rs`)

- WAV uses `hound`; MP3 / FLAC / OGG / AAC and others use `symphonia`, all producing interleaved f32 PCM.
- It does file IO and decodes the whole file, so it **must only run on the control side**, never on the real-time thread.

---

## 7. audio-engine: three executors and the clock

**Location**: `crates/audio-engine/src/`

### Three executors (`executors.rs`)

The engine doesn't hard-code threads; it only defines **logical roles**:

| Trait | Job | Where it really runs |
|---|---|---|
| `ControlExecutor` | handles commands, device switches, parameter validation | app thread |
| `DecodeExecutor` | file decoding, varispeed, prefilling rings | a background thread |
| `RealtimeCallback` | drains commands (≤32), fetches PCM, mixes / DSP, writes out | the cpal output callback |

`EngineCommand`: the command enum (set gain, set rate, start / stop sources, switch device, shut down).

### `DriftController` (`clock.rs`)

Fill-level PI drift control: the ring buffer's **fill level** is the controlled variable, the PI output is limited
to ±1000 ppm and smoothed by a one-pole low-pass filter.

```rust
pub fn update(&mut self, current_fill: u32);  // called periodically from a non-real-time thread
pub fn ratio(&self) -> f32;                    // read-only on the real-time side
```

- `ratio_bits`: the f32 is shared across threads as the bit pattern in an `AtomicU32` (no non-atomic f32).
- **Why not estimate the ratio from accumulated sample counts?** f32 precision isn't enough, and it's open-loop
  without feedback. Fill-level PI is closed-loop.

### `Engine` (`engine.rs`)

Engine runtime state: mix parameters, linear gains, file rate, running flag. Owned exclusively by the real-time thread.
- `apply_command(cmd)`: applies a command on the real-time side.
- `mix(file, mic, out, frames, channels)`: adds the two paths + `tanh` soft limiting.

---

## 8. application: assembly layer

**Location**: `crates/application/src/`

### `Mixer` (`mixer.rs`)

A real-time safe mixer. Gains / rate are shared across threads as `Arc<AtomicU32>` (f32 bit patterns).
- `mix(file, mic, out)`: `file*fg + mic*mg`, then `tanh` soft limiting.
- `set_file_gain_db` / `set_mic_gain_db` / `set_file_speed`: called on the control side, atomic writes.

### `App` (`app.rs`)

The assembly core: wires the backend, the mixer and the file source together.

**Threads in `start()`**:
1. Open the output stream: the real-time callback reads two SPSC rings → mixes → writes out.
2. Open the input stream: the mic callback writes the mic ring.
3. Spawn the decode thread: the file source's `fill` → channel conversion → writes the file ring.

**`RunHandle`**: holds the stop flag, the decode thread and the stream handles; on drop it stops, joins and releases the streams.

**`convert_channels(src, src_ch, dst, dst_ch)`**: channel conversion (mono ↔ stereo etc.) so file and engine channel counts match.

---

## 9. audio-cpal: hardware backend

**Location**: `adapters/audio-cpal/src/lib.rs`
**Role**: the cpal implementation of `AudioBackend`, for all platforms (Windows WASAPI / macOS CoreAudio / Linux).

### `CpalBackend`

- `new()`: creates the default host with `cpal::default_host()`.
- `enumerate_devices()`: enumerates devices and collects supported input / output configs.
- `open_output(config, callback)` / `open_input(...)`: build the stream and `play()` it.

**Key details**:
- `pick_rate`: if the requested rate is outside the device's range, falls back to the highest supported value.
- `to_buffer_size`: `0` → the device's default buffer, otherwise a fixed frame count.
- The callback calls the user callback directly, with no extra copy.

### Opening streams with the device's native config (used by the soundboard)

```rust
let out = backend.resolve_output(Some("CABLE Input"))?; // or None = default device
let cfg = out.config();          // the device's native sample rate / channels
let handle = backend.build_output(out, 512, callback)?;
```

- `resolve_output(name)` / `resolve_input(name)`: find a device by case-insensitive substring match, `None` for the
  system default; returns a `ResolvedDevice` (device + name + native config).
- **Why resolve before opening**: WASAPI shared mode only accepts the device's mix format. Knowing the real rate first
  lets the caller prepare its callback state (the soundboard's rate conversion, the mic resampling).
- `build_output(dev, buffer_frames, cb)`: `buffer_frames` is only a hint — the callback may receive a different number
  of frames and must handle any length.
- `build_input(dev, cb, on_error)`: `on_error` is called when the stream fails (e.g. a Bluetooth headset disconnects),
  which is how the mic thread notices and reconnects.

---

## 10. desktop: CLI shell

**Location**: `apps/desktop/src/main.rs`
**Role**: a GUI-less command-line demo that verifies the end-to-end path.

```text
soundbridge --list                          list devices
soundbridge <file.wav> --speed 1.0 --file-gain 0 --mic-gain 0   play a file mixed with the mic
```

- `main()`: parse arguments → `App::new` → `load_file` → set gains / speed → `start` → wait for Ctrl-C → `stop`.
- `ctrlc`: catches Ctrl-C for a graceful shutdown.

---

## 11. soundboard: soundboard core

**Location**: `crates/soundboard/src/lib.rs`
**Role**: manages sound definitions and playing instances, and mixes them in the output callback. See
[Soundboard architecture](soundboard-architecture.md) for the overall design.

### `LoadedSound`

```rust
let loaded = LoadedSound::load(spec)?;          // control side: read file + decode
cmd_tx.send(BoardCommand::AddSound(Box::new(loaded)))?;
```

- "Read file + decode" happens on the control side (the loader thread in the server), so the real-time thread receives ready PCM.
- Load failures (missing file, unsupported format) return an error right here, so the API can tell the user directly.
- `decoded_bytes()`: memory used after decoding; the loader uses it to cap the preload total.
- `LoadedSound` is `Clone`: it only copies the `Arc`-shared PCM, which is how decoded sounds are reinstalled into a new
  soundboard after an output switch; `with_spec(spec)` swaps in the latest settings.

### `PlayMode`

- Two independent booleans `{ looping, exclusive }`, replacing the old one-of-four `TriggerMode`.
- `From<TriggerMode>`: converts old data (`Loop` → looping, `Exclusive` → exclusive, anything else → both off).
- Trigger logic: looping and already playing → stop (press again to stop); exclusive → clear the other instances first; then create a new instance.

### `SoundBoard`

- `new(capacity_samples, output_rate)`: preallocates scratch, the instance table (`MAX_INSTANCES`) and the definition table.
- `apply_command(cmd)`: runs commands inside the output callback. `Trigger` only clones shared PCM, no allocation;
  `SetMode` updates the definition and changes the loop flag of playing instances in place (no interruption);
  `Seek` sets the position of all instances of that sound.
- `for_each_playing(|id, progress|)`: calls back for each playing instance in start order, for `PlayingSet` to publish to the UI.
- `fill(out, frames, channels)`: overwrites `out`. Each instance steps by `speed × rate_ratio`; chunks when scratch is too small.
- `fill_instance` / `accumulate`: associated functions (not borrowing `self`), so callers can borrow the `instances`
  and `scratch` fields at the same time.

---

## 12. server: soundboard service library

**Location**: `apps/server/src/` (library `soundbridge_server` + browser entry `soundbridge-server`)

| File | Contents |
|---|---|
| `lib.rs` | `ServerConfig` (with the host `Hooks`), start-up `run()`, `Control` (close prompt), embedded front-end `static_handler`, `lan_ip`, `is_audio_file` |
| `main.rs` | browser entry: `ServerConfig::from_env()` → `run()` |
| `api.rs` | `AppState` + every HTTP handler; `apply_diff` syncs scan results to loading and hotkeys; `open_in_browser` |
| `library.rs` | `Library`: library = folder on disk (scan, folder ops, `.soundbridge.json`, v1 migration); `SoundDefaults` |
| `loader.rs` | `Player`: the loader thread (preload + on-demand) and the single trigger entry |
| `audio.rs` | `start_audio`: the output thread (open / switch the output), the output callback; `describe_output`, `MicReader`, `PlayingSet` |
| `mic.rs` | `MicControl` (atomic parameters + status), the mic thread, `auto_pick`, `MicCapture`, `input_risk` |
| `monitor.rs` | monitor: `MonitorControl`, the monitor thread, `MonitorTap` (main callback side), `MonitorReader` (monitor callback side) |
| `remote.rs` | `RemoteServer`: start / stop the phone remote listener, access-code middleware `require_token`, `qr_svg` |
| `settings.rs` | `SettingsStore`: reads / writes `settings.json`; `data_dir()` (per platform); `CloseAction`; `CustomSite`, `is_web_url`; rename migration `migrate_legacy_data_dir` |
| `i18n.rs` | UI language: `apply` / `is_en`, macros `tr!` / `trf!` produce messages and errors in the current language |
| `hotkeys.rs` | global hotkey thread (Win32 message loop + bind requests); calls `Player::trigger` on key press |
| `virtual_mic.rs` | Linux virtual mic: create / take over / unload modules with `pactl` |

### Start-up order (`run`)

1. Run the rename migration (`migrate_legacy_data_dir`), read settings (`SettingsStore`), apply the UI language
   (`i18n::apply`), decide the output device: `ServerConfig.output_device` > last choice > system default.
2. On Linux with no output device specified, `VirtualMic::setup()` creates the virtual mic and sets
   `PULSE_SINK` / `PIPEWIRE_NODE` (must happen before any audio stream opens).
3. `migrate_v1` migrates old data; `Library::open_with(root, sound_defaults)` opens and scans the library.
4. `Player::new` starts the loader thread; `MicControl` and `MonitorControl` are created;
   initial mic: `ServerConfig.input_device` > last choice > auto.
5. `audio::start_audio(...)`: starts the output thread (opens the output device, builds the output callback) →
   `spawn_mic_thread` → `spawn_monitor_thread`.
6. Start the hotkey thread; bind every sound's hotkey and queue preloading (start-up isn't blocked).
7. The main UI listens on `127.0.0.1` only and binds the port (the app may fall back to a random port).
8. Create `RemoteServer` (access code from settings, generated if missing) on main port + 1;
   start it immediately if `lan_remote` is set or `SOUNDBRIDGE_LAN=1`.
9. Assemble `AppState` (incl. `hooks` and `close_prompt`), hand the router to `RemoteServer`, call `on_ready` to notify the host, and serve.

**The output thread**: cpal stream handles can't cross threads on some platforms, so the main output stream is owned
by the dedicated thread `soundbridge-output`, which also closes the old stream and opens the new one on a switch
(`OutputRequest::Select { device, reply }`). The host still runs `block_on(run(...))` on its own thread.

### `RemoteServer` (`remote.rs`)

- `start()`: binds `0.0.0.0:preferred_port` (a random port if taken) and serves the same `Router` with
  `with_graceful_shutdown`; `stop()` sends the shutdown signal and the port is released. No restart either way.
- `require_token`: the `t` query parameter or the `sb_token` cookie must equal the access code (a `t` parameter sets
  the cookie); endpoints in the `DESKTOP_ONLY` list allow only GET from phones.
- `status()`: `{enabled, url, port, error}`, with `url` = `http://<LAN IP>:<port>/?t=<code>`.

### Host hooks and the close prompt (`lib.rs`)

- `Hooks { on_settings, on_window_action, on_open_url }`: the service library doesn't depend on Tauri; the desktop app
  receives "settings changed" (to tint the icon and rebuild the tray menu), "close prompt answered" (hide to tray /
  quit) and "open this URL" (the preview window) through hooks.
- `Control::request_close()`: raises `close_prompt` and returns whether it was already raised — clicking × again while
  the prompt is pending means the UI isn't responding, so the shell quits. The UI sees the flag via `/api/live`, shows
  the dialog, and answers with `POST /api/window`, which clears it.

### `Library` (`library.rs`)

- `open_with(root, defaults)`: creates the directory, reads `.soundbridge.json`, runs `scan`; `set_defaults` changes
  the initial settings for sounds found later.
- `scan`: recursively collects folders and audio files → assigns ids to new files (with `SoundDefaults`) → drops entries
  that no longer exist → writes the settings file back → returns `ScanDiff { added, removed }`.
- `create_folder` / `rename_folder` / `delete_folder`, `import` / `rename_sound` / `move_sound` / `delete_sound`:
  change the disk first, then move the settings keys (`rekey_prefix` / `move_entry`), then `scan`. Every input path goes through `clean_rel`.
- `set_root(new, move_contents)`: optionally moves the old library's contents (`move_dir_contents`, copy + delete across drives).
- `migrate_v1(data_dir, root)`: old `library.json` → new library, see architecture §7.1.

### `Player` (`loader.rs`)

- `trigger(id)`: sends `Trigger` directly if loaded; otherwise queues `LoadAndTrigger` on the priority queue.
- `preload(id)` / `install(loaded)` / `unload(id)` / `unload_all()` / `send(cmd)`.
- The loader thread `Worker::run`: takes the priority queue first, then waits on both queues with `select!`; skips
  preload jobs once the preloaded total exceeds `PRELOAD_BUDGET_BYTES`. It always takes the latest definition from `Library::spec` before decoding.
- `reinstall_all()`: after an output switch, puts every cached decoded sound back into the new soundboard.

### `PlayingSet` (`audio.rs`)

- After every `fill` the output callback calls `publish(&board)`, writing (sound id, progress) of the playing
  instances into two fixed sets of atomic slots; id 0 means empty.
- `/api/live` reads `snapshot()`: one item per sound, the newest instance's progress when several play. Lock-free, allocation-free, real-time safe.

### Hotkey thread (`hotkeys.rs`)

- `Bindings::bind`: parse → check whether another sound uses it → register the new key → release the old key → update the table.
- `pump_messages`: dispatches `WM_HOTKEY` on Windows (without it hotkeys never fire); empty on other platforms.

### `MicControl` (`mic.rs`)

- `gain_db` / `muted` / `level`: atomics read and written directly by the real-time callbacks.
- `status: Mutex<MicStatus>`: device status (selection, device in use, connected, note), locked **only on the control side**.
- `update_level(peak)`: fast attack, slow release (decays to 0.9× per callback), for the UI level meter.

### Mic thread (`spawn_mic_thread`)

```
loop {
    recv_timeout(1s) {
        Select{device}  → open(device): replace the old stream only on success + remember; reply to the API
        OutputChanged   → reopen at the new output rate, redo the feedback check
        timeout         → watch(): reconnect if disconnected; in auto mode, follow default-device changes
    }
    free the old rings sent back by the output callback
}
```

- `open(device)`: resolve the device → `input_risk` check → new ring + `MicCapture` → `build_input` → pass the
  consumer to the output callback via `cons_tx` → replace the old stream. Any failure keeps the old stream.
- `set_unavailable(note)`: logs the same reason only once, instead of every second.
- `auto_pick()`: with "Auto", uses the system default recording device if it's a hardware mic, otherwise the first hardware mic.

### `input_risk(input, output)` (input device check)

- `Some(InputRisk::FeedbackLoop)`: the input is the other end of the virtual device being output to, so the output would be recorded back.
- `Some(InputRisk::SystemLoopback)`: the input is a loopback device like Stereo Mix, which would record everything that's playing.
- **Why only names**: no platform has a common API that says "this recording endpoint belongs to that playback
  endpoint", while virtual cables are named very regularly ("endpoint (driver)", both ends with the same name, …).
- **Why require "virtual"**: a sound card's mic and speakers also share a driver name (e.g. `Realtek(R) Audio`), and a
  Bluetooth headset's mic and earpiece can even have identical names (e.g. `Headphones (AirPods)`), yet none of them
  loop — so only names with virtual keywords count as feedback loops.
- Rules and keyword lists: the constants at the top of `mic.rs` and [Soundboard architecture §6.2](soundboard-architecture.md).

### `MicCapture` (input callback side) and `MicReader` (output callback side)

- `MicCapture::push`: downmix to mono → linear-interpolation resampling (`step = input rate / output rate`) → write to the ring.
- `MicReader::mix_into`: pick up a new ring (device switch) → drift trimming → jitter buffer (primed) → add to each channel with the gain → update the level.

### Monitor (`monitor.rs`)

- `MonitorTap::tap` (main output callback, before the mic is added): writes the effects-only buffer as stereo into the
  ring; it first picks up any new ring from the monitor thread and returns early when there's none (off or not needed).
- `MonitorReader::fill` (monitor callback): pick up a new ring → drift trimming (drop backlog > 60 ms) → jitter buffer
  (about 20 ms) → linear interpolation with `step = main rate / monitor rate` → apply the monitor volume. The first frame
  serves as both interpolation points, avoiding a zero sample at the start.
- The monitor thread: `Configure { enabled, device }` toggles / changes the device; on `MainChanged` (the main output
  changed) it decides again whether monitoring is needed (not when the main output is a speaker or equals the monitor device).

---

## 13. app: desktop app (Tauri shell)

**Location**: `apps/app/`

| File | Contents |
|---|---|
| `src/main.rs` | Tauri entry: single-instance plugin, starting the service (with hooks), windows, tray, close prompt, icon tint, the sound-site preview window |
| `tauri.conf.json` | product name, identifier, icons, release bundles (NSIS / deb / AppImage); windows are created in code (`windows: []`) |
| `loading/index.html` | error page shown when the service fails to start |
| `loading/titlebar.html` | the custom title bar of the preview window |
| `preview/preview.js` | download notices injected into sites in the preview window |
| `capabilities/preview-titlebar.json` | the only Tauri permissions in the app: drag and window buttons, for the preview title bar only |
| `icons/` | app icons (`icon.ico` holds 16/32/48/256 sizes) |

- `start_server()`: spawns a thread → builds a tokio runtime → `block_on(run(config, on_ready))`, sending the port
  back to the main thread over an `mpsc` channel, waiting at most 20 s.
- `setup`: when the service is ready, the window loads `WebviewUrl::External("http://127.0.0.1:<port>")`, otherwise the bundled error page.
- Window: 1080×700, minimum 760×520, background colour matching the UI (no white flash), Tauri's drag-and-drop interception off.
- `build_tray`: tray icon + menu (open / quit, in the UI language; rebuilt by `apply_tray_menu` on `on_settings`); left-click runs `show_main`.
- `on_window_event(CloseRequested)`: for the main window only — hide / quit / ask (`Control::request_close`) according to `close_action`.
- `open_browse_window(url, name)`: the sound-site preview window, frameless, with two webviews: the local title bar
  (`browse-bar`) and the site (`browse-site`); an existing window is reused with `navigate`. `layout_browse_window`
  re-lays them out on resize. The title bar's `on_navigation` catches the reserved `ferrymic-open.invalid` address and
  opens the site's current URL in the system browser.
- `handle_download`: in the preview window, audio files (`is_audio_file`) are saved to the library's "Downloads" folder;
  a file already there with the same name cancels the download (remembered in `skipped_downloads` so the cancel isn't
  reported as a failure); start / finish / skip / failure notices are shown via `js_str` + `eval`.
- `tinted_icon(accent)`: converts the bundled `icons/icon.png` pixel by pixel to HSL and rotates the hue by
  "accent hue − 246.1°", leaving very low-saturation pixels (the white glyph) alone; hooks fire on a background thread,
  so icons are set on the main thread (`run_on_main_thread`).
- `#![windows_subsystem = "windows"]`: no console window in release builds.
- Building: Windows uses `build-app.ps1` (front-end → `cargo build --release -p soundbridge-app` → copy the exe and
  `WebView2Loader.dll` into `dist-app/FerryMic/`); Linux uses `build-app.sh` (same steps, a single executable). Release
  packages are built by GitHub Actions (`.github/workflows/release.yml`).

---

## Appendix: naming conventions

| Convention | Meaning |
|---|---|
| `*_db` | value in dB (logarithmic) |
| `*_lin` | linear value (amplitude) |
| `frames` | frame count (one frame = one sample per channel) |
| `samples` | sample count (frames × channels) |
| `prod/cons` | ringbuf producer / consumer |
| `ring` | SPSC ring buffer |
