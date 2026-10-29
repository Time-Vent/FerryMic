# Soundboard Architecture

**English** | [简体中文](../soundboard-architecture.md)

> This document describes the technical architecture of the soundboard: mixing the mic with sound
> effects, output switching and monitoring, the sound library, hotkey triggering, the desktop app
> (tray, close prompt, icon tint), the UI and the phone remote.

## 1. Purpose

The soundboard is FerryMic's **core product**: it captures the real microphone, manages many sound
effects with hotkeys, and mixes "your voice + the effects" into a sound card / virtual audio cable.
Target apps (voice chat, meetings, streaming) use the virtual cable as their microphone, so the other
side hears both the voice and the effects.

## 2. Modules

Two hosts share one service library and the same UI:

```
apps/app (desktop app, Tauri)        apps/server/src/main.rs (browser version soundbridge-server)
  └─ in-process service + native window   └─ starts the service, used from a browser
                 \                         /
                  apps/server (library soundbridge_server)
                    ├─ lib.rs        start-up run(), listen address, embedded front-end (rust-embed)
                    ├─ api.rs        HTTP API handlers + AppState
                    ├─ library.rs    library = folder on disk (scan, folder ops, .soundbridge.json, v1 migration)
                    ├─ loader.rs     loader thread: background preload + on-demand load; Player = the single trigger entry
                    ├─ audio.rs      output thread: SoundBoard mix + monitor tap + mic + limiter; runtime output switching; PlayingSet (with progress)
                    ├─ mic.rs        mic thread: auto hardware mic / switching / reconnect, input device checks
                    ├─ monitor.rs    monitor thread: effects-only mix → a second output stream (headphones / speakers)
                    ├─ remote.rs     phone remote: LAN listener (port + 1), access code, QR code, on/off at runtime
                    ├─ settings.rs   settings persistence (devices, monitor, remote, appearance, window, library location) + data directory
                    ├─ i18n.rs       UI language: tr! / trf! for user-facing messages
                    ├─ hotkeys.rs    global hotkey thread (Win32 message loop)
                    ├─ virtual_mic.rs Linux virtual microphone (pactl)
                    └─ (depends on) soundboard crate → audio-source (decoding + shared PCM + varispeed + seeking)
apps/web (Vite + TS, no framework)  the UI; its build output is embedded into the service library
```

```
crates/soundboard/
  ├─ PlayMode       playback: two independent switches { looping, exclusive } (the old TriggerMode is only read from old data)
  ├─ SoundSpec      sound definition: name / file / play mode / gain / speed / hotkey
  ├─ LoadedSound    decoded sound = SoundSpec + shared PCM (decoded on the control side)
  ├─ BoardCommand   commands: AddSound(LoadedSound) / Trigger / Stop / SetGain / SetSpeed / SetMode / Seek ...
  ├─ Instance       a playing instance (cloned VarispeedSource + gain / speed / rate ratio)
  └─ SoundBoard     the board: owns definitions and instances, mixes in fill (owned by the real-time thread)
```

## 3. Threads

| Thread | Job | Real-time |
|---|---|---|
| tokio pool | axum server, REST API; decoding, device enumeration and waiting for replies run in `spawn_blocking` | no |
| loader `soundbridge-loader` | decodes sounds for the audio thread; trigger requests jump ahead of preloading | no |
| output `soundbridge-output` | owns the main output stream (cpal stream handles can't cross threads); handles output switching | no |
| mic `soundbridge-mic` | owns the input stream; handles switching; checks every second for disconnects / default-device changes | no |
| monitor `soundbridge-monitor` | owns the monitor output stream; handles on/off and device changes; reopens when the main output changes | no |
| hotkeys `soundbridge-hotkeys` | creates the hotkey manager + Win32 message loop; handles bind requests, sends Trigger on key press | no |
| cpal input callback | mic → downmix to mono → resample to the output rate → write to an SPSC ring | **yes** |
| cpal output callback | drain commands (≤32) → SoundBoard::fill → publish playing state → monitor tap → add mic → limit → write | **yes** |
| cpal monitor callback | read the monitor ring → interpolate by the rate ratio → apply monitor volume → write to headphones / speakers | **yes** |
| (app) Tauri main thread | native windows and WebView2, tray, icon tint; the service runs on a tokio runtime in its own thread | no |

**Data flow**:

```
microphone (phone mic / Bluetooth headset / USB) ─input callback─▶ mono + resample ─SPSC ring─┐
                                                                                             ▼
Web API / global hotkey ──Player──▶ (decoded by the loader if needed) ──BoardCommand──▶
    output callback: SoundBoard mix ──┬──▶ + mic ─▶ virtual cable (other apps pick its other end as the mic)
                                      └─monitor ring─▶ monitor callback ─▶ headphones / speakers (effects only)
```

**Real-time safety**: the input / output callbacks never allocate, lock or do IO. Audio files are
decoded by the loader thread (`LoadedSound::load`); triggering only clones the `Arc`-shared PCM on the
real-time thread, and seeking only changes an instance's phase.

## 4. Play modes

The old four trigger modes (one-shot / loop / exclusive / overlap) were really two independent
choices, now split into two switches `PlayMode { looping, exclusive }`:

| Switch | On | Off |
|---|---|---|
| `looping` (double-arrow icon on the card, with a "1" when off) | Loop: press to start looping, press again to stop | Once: each press plays from the start and stops at the end |
| `exclusive` (icon on the card: exclusive = dot, overlap = two overlapping circles) | Exclusive: starting it stops every other sound | Overlap: plays alongside other sounds |

They combine freely (e.g. "exclusive + loop" for background music). When `BoardCommand::SetMode`
changes the setting, playing instances update their loop flag in place without interruption.

- **Old data**: a legacy `trigger_mode` in `.soundbridge.json` is converted on load: `OneShot` /
  `Overlap` → both off, `Loop` → looping, `Exclusive` → exclusive; only the new `mode` field is written back.

Each sound is configured independently: play mode + gain (dB) + speed (varispeed: speed and pitch
change together). At most `MAX_INSTANCES = 32` instances play at once (preallocated); when full, the
oldest one is replaced.

## 5. Mixing (SoundBoard::fill)

- Each playing instance runs `VarispeedSource::fill` into a scratch buffer, which is added to the output with its gain.
- **Rate conversion**: step = user speed × source rate / output rate, so a 44.1 kHz file doesn't change pitch on a 48 kHz output.
- **Chunking**: when the driver asks for more frames than the scratch buffer holds, it's processed in chunks, never out of bounds.
- Source channels map to output channels (mono duplicated / stereo direct / otherwise the overlap).
- Non-looping instances are removed when exhausted; looping ones reset and continue.
- **Progress and seeking**: `for_each_playing` reports (id, progress 0..1); the output callback writes it into
  `PlayingSet` (a fixed set of `MAX_INSTANCES` atomic slots) every time; `BoardCommand::Seek` sets the phase of all
  instances of that sound. With several instances of one sound, the UI shows the newest one's progress.

## 6. Microphone

### 6.1 Capture and mixing

- **Input callback** (`MicCapture`): downmixes to mono, linearly resamples to the output rate, writes to an SPSC ring.
  Bluetooth hands-free (8/16 kHz) and phone-mic apps (usually 44.1/48 kHz) go through the same conversion.
- **Output callback** (`MicReader`): reads the ring and adds it to every output channel with the mic gain.
  - Jitter buffer: output starts only once about 15 ms has accumulated, absorbing the two devices' callback timing.
  - Drift trimming: when the backlog exceeds the 60 ms target, the excess is dropped so latency doesn't keep growing.
  - Underrun / disconnect: silence and re-buffer automatically.
- Muting only stops sending; the level meter still shows the input, so you can see the mic is working.
- The final output is hard-limited to [-1, 1].

### 6.2 Switching at runtime (`mic.rs`)

Mic sources change often (phone mic apps, Bluetooth headsets connecting and disconnecting), so the
input stream lives on a dedicated mic thread:

- **Switching**: `POST /api/mic/device` → `MicRequest::Select` → the thread opens the new device and replaces the
  old stream **only on success**; on failure the API returns 400 and the old device stays.
- **Ring hand-over**: every new stream gets a new SPSC ring whose consumer is passed to the output callback over a
  lock-free channel; the output callback sends the old ring back to the mic thread to be freed, so the real-time
  thread never frees large blocks.
- **Disconnect detection**: a stream error (`on_error` of `build_input`) sets a flag; the thread checks every second
  and reconnects with the same selection, showing "Waiting for the microphone…". Bluetooth headsets recover by themselves.
- **Auto (default)**: with "Auto", the system default recording device is used if it's a hardware mic, otherwise the
  first hardware mic ("hardware" = not a virtual cable, not a loopback device, no loop with the current output).
  When the default device changes (e.g. a phone mic connects), the thread follows it.
- **After an output switch**: the output thread sends `MicRequest::OutputChanged`; the mic thread reopens at the new
  rate and re-runs the feedback check.
- **Input device checks** (`input_risk`, ADR-0006): the device name is checked before opening and matching devices are
  refused. Inputs in `GET /api/devices` carry a `virtual` flag, and the UI groups virtual devices separately. The rules
  work across vendors:

  | Case | Verdict | Examples |
  |---|---|---|
  | Name contains a loopback keyword | loopback device: records everything that's playing | `Stereo Mix`, `立体声混音`, `What U Hear`, Linux `Monitor of …` |
  | Same name as the output, and virtual | feedback loop | VAC `Line 1 (Virtual Audio Cable)`, macOS `BlackHole 2ch` |
  | Same driver name in parentheses, and virtual | feedback loop | `CABLE Input/Output (VB-Audio Virtual Cable)`, `VoiceMeeter Input/Output (VB-Audio VoiceMeeter VAIO)` |
  | No parentheses, same name after removing direction words, and virtual | feedback loop | `… Virtual Source` / `… Virtual Sink` |

  "Virtual" is decided by name keywords (`virtual`, `cable`, `vb-audio`, `voicemeeter`, `vaio`, `blackhole`,
  `soundflower`, `loopback`), so a sound card's own mic and speakers (e.g. `Realtek(R) Audio`) and a Bluetooth
  headset's identically named mic and earpiece (e.g. `Headphones (AirPods)`) aren't misjudged. Name heuristics
  can't cover every device; unusually named cables may slip through.

### 6.3 Settings (`settings.rs`)

- File: `settings.json` in the data directory. Data directory: Windows `%APPDATA%\FerryMic`,
  Linux `$XDG_DATA_HOME/FerryMic` (default `~/.local/share/FerryMic`),
  macOS `~/Library/Application Support/FerryMic`; overridable with `SOUNDBRIDGE_DATA_DIR` (for tests).
- Contents:
  - devices: `mic_device` (`null` = auto), `mic_gain_db`, `output_device` (`null` = system default);
  - monitor: `monitor_enabled` (on by default), `monitor_device` (`null` = default playback device), `monitor_gain_db`;
  - phone remote: `lan_remote`, `remote_token` (access code, generated on first start);
  - appearance: `theme_mode` (system / dark / light), `accent` (`#rrggbb`), `tint_app_icon` (on by default),
    `language` (`system` / `zh-CN` / `en`, default `system`);
  - window: `close_action` (`Ask` / `Tray` / `Quit`, default `Ask`);
  - other: `library_dir` (`null` = `sounds/` in the data directory), `auto_virtual_mic` (Linux);
  - defaults for new sounds: `sound_defaults` (`{looping, exclusive, gain_db, speed}`; default exclusive, once, 0 dB, 1.0×);
  - sound sites: `custom_sites` (user-added sites `[{name, url}]`).
  Fields missing from older files take their defaults.
- **Rename migration** (`migrate_legacy_data_dir`): at start-up, if the old data directory `SoundBridge` exists and
  `FerryMic` doesn't, it's renamed over; a `library_dir` inside the old directory is moved with `rebase_path`.
  No migration when `SOUNDBRIDGE_DATA_DIR` is set.
- Mic priority at start-up: `SOUNDBRIDGE_INPUT_DEVICE` > last choice > auto.
  Output: `SOUNDBRIDGE_OUTPUT_DEVICE` > last choice > system default.
- Mute isn't saved (its start value comes from `SOUNDBRIDGE_MIC_MUTED`).

## 7. Sound library (`library.rs` + `loader.rs`)

### 7.1 A folder on disk is the library

- **The root** comes from settings; audio files under it are the sounds and **subfolders are categories** (at most
  8 levels and 10,000 files; entries starting with `.` are ignored). Supports WAV / MP3 / FLAC / OGG / M4A / AAC.
- **Per-sound settings** live in `.soundbridge.json` at the root:
  `{ version: 2, next_id, sounds: { "relative/path": {id, mode: {looping, exclusive}, gain_db, speed, hotkey} } }`,
  keyed by `/`-separated relative path, so the whole folder can be moved / synced. Writes go to a temp file + rename.
- **Name = file name without extension**: renaming in the app renames the file, moving moves the file, and the settings
  follow; renaming / moving in the file manager looks like delete + add, and the settings are lost.
- **Scanning** (at start-up, `POST /api/library/rescan`, and when the window regains focus): syncs entries and settings
  and returns added / removed ids, which the API layer uses to unload / preload sounds and unbind / bind hotkeys.
- **Initial settings for new sounds**: files seen for the first time get settings from `SoundDefaults` (the
  `sound_defaults` setting; changes apply at once via `Library::set_defaults`); existing sounds are untouched.
  "Apply to all existing sounds" in Settings is done by the UI with one PATCH per sound; hotkeys, names and folders don't change.
- **ids**: stored in the settings file and stable across restarts (hotkeys depend on them); new files get new ids,
  `next_id` only grows; duplicate ids from hand-edited files are reassigned.
- **Folder operations**: create, rename / move (not into its own subfolder), delete (only folders without sounds).
  Every relative path goes through `clean_rel`, which strips `..`, absolute paths and invalid characters to prevent path traversal.
- **Import** (`POST /api/sounds/import`, multipart: `file` + optional `folder`): written into the folder, numbered on
  clashes (`name (2).mp3`), then decoded to validate; on failure the file is removed and 400 returned.
- **Delete**: the file goes to the system recycle bin (`trash` crate; the freedesktop trash on Linux), or is deleted where that isn't supported.
- **Change location** (`POST /api/library/location`): optionally "move the existing files too" (rename on the same
  drive, copy + delete across drives, error if the target already has an item of the same name); afterwards all sounds
  of the old library are unloaded and hotkeys unbound, then the new library loads.
- **v1 migration** (`migrate_v1`): on first start, if the data directory has an old `library.json`, files are renamed to
  "name.ext" in the root, their settings (incl. ids and hotkeys) written to `.soundbridge.json`, and the old file renamed to `library.v1.json.bak`.

### 7.2 Loading (`loader.rs`)

A library may point at a huge folder, and decoding everything into memory isn't realistic (3 minutes of stereo ≈ 66 MB):

- **The loader thread** preloads in order and stops preloading once the decoded total exceeds `PRELOAD_BUDGET_BYTES` (768 MB).
- Sounds not yet loaded are decoded **on first trigger** and play right after; trigger requests use a priority queue, not behind preloading.
- `Player::trigger` is the single trigger entry for both the Web API and global hotkeys, so hotkeys can trigger unloaded sounds too.
- Changing volume / speed / play mode sends commands to the audio thread for loaded sounds; unloaded ones read the latest settings when loaded.
- **On an output switch** the new `SoundBoard` is empty, and `Player::reinstall_all` puts the cached decoded sounds back
  (cloning a `LoadedSound` only copies the `Arc`, nothing is decoded again).
- Runtime info (duration, loaded, load error) lives only in memory; the UI shows durations from it and refreshes every 3 s while sounds are still unloaded.

## 8. Global hotkeys (`hotkeys.rs`)

- Uses `global-hotkey` (Windows RegisterHotKey / macOS / Linux X11).
- **Windows needs a message loop**: `WM_HOTKEY` is only posted to the thread that created the hotkey manager, so the
  hotkey thread dispatches messages with `PeekMessageW/DispatchMessageW` every 10 ms — without it hotkeys never fire.
- Binding is request / reply (`HotkeyCommand::Bind { reply }`): the new key is registered first and the old one released
  only on success; an unrecognised hotkey, one used by another sound, or one taken by another program returns an error
  (the API returns 400) and the old key stays.
- Hotkey strings are parsed by `HotKey::from_str`; the UI records them from the browser's `KeyboardEvent.code` in the form
  `Ctrl+Alt+KeyQ`. A lone letter / digit would swallow typing system-wide, so the UI requires a modifier.
- **Two kinds** (chosen per sound, `hotkey_passthrough` in `.soundbridge.json`):
  - **Blocking (default)**: a system hotkey via `RegisterHotKey`. The foreground program doesn't receive the key, and only
    Ctrl / Alt / Shift / Win can be modifiers.
  - **Pass-through (in-game combo, Windows)** (`passthrough.rs`): low-level hooks `WH_KEYBOARD_LL` / `WH_MOUSE_LL` only
    observe and always pass on with `CallNextHookEx`, so "push-to-talk + 1" can play a sound in FerryMic and switch
    weapons in the game at once. Combos are key names in press order (the last is the trigger, the rest must be held),
    e.g. `KeyV+Digit1`, `Mouse4+Digit2`; holding extra keys doesn't prevent a match.
    - The hooks live on a dedicated thread `soundbridge-combo-hook` with a blocking `GetMessageW` loop (low-level hook
      callbacks run through it) and start when the first pass-through combo appears. The callback only looks up the table
      (`try_read`; if the lock is busy it skips) and sends the sound id over a channel; the forwarding thread
      `soundbridge-combo` calls `Player::trigger`, so system-wide input is never held up.
    - Both kinds share one binding table; conflicts are checked by an order-independent signature (the same keys can't belong to two sounds).
    - Limits: input to a program running as administrator isn't seen while it's in front; Linux / macOS aren't supported
      yet (the API returns an error and the UI hides the option).

## 9. Desktop app (`apps/app`)

- **Tauri 2 shell**: shows the UI in the system WebView (WebView2 on Windows, WebKitGTK on Linux) without bundling a
  browser engine. The Windows release exe is about 10 MB; a GNU-toolchain build also needs `WebView2Loader.dll`
  next to it (`build-app.ps1` copies it); Linux builds with `build-app.sh`. Release packages come from CI (see the README).
- **In-process service**: `soundbridge_server::run` runs on a tokio runtime in its own thread; once ready, the window
  loads `http://127.0.0.1:<port>`. The UI is identical to the browser version and talks HTTP, **not Tauri IPC**, so the
  main window needs no Tauri permissions.
- **Port**: 8321 by default; if it's taken (e.g. the browser version is running), a random port is used.
- **Single instance**: `tauri-plugin-single-instance`; launching again just brings the existing window to the front, avoiding two audio outputs.
- **Drag-and-drop import**: Tauri's file-drop interception is disabled (`disable_drag_drop_handler`) so the page handles HTML5 drops.
- **Start-up failure**: shows a bundled error page (`apps/app/loading/index.html`) with the message passed as a query parameter.
- **Tray**: `TrayIconBuilder` creates the tray icon with an "Open FerryMic / Quit" menu (in the UI language); left-click restores the window.
- **Close prompt**: `CloseRequested` on the main window is handled per the `close_action` setting:
  - `Tray`: hide the window; `Quit`: exit;
  - `Ask` (default): `Control::request_close` raises a flag; the UI sees `close_prompt` while polling `/api/live` and shows
    an in-page dialog ("Minimize to tray / Quit / Cancel" + "Remember my choice"); the answer goes back through
    `POST /api/window` → the `on_window_action` hook. An in-page dialog matches the theme; if it doesn't respond
    (e.g. the UI hangs), clicking × again quits.
  - Other windows (the sound-site preview) just close.
- **Icon tint**: when the accent changes (`on_settings` hook), the bundled `icons/icon.png` is hue-rotated by
  "accent hue − base hue 246.1°", skipping low-saturation (white) pixels, and set as the icon of every window and the tray.
  With `tint_app_icon` off, the original icon is restored.
- **Hooks**: the service library doesn't depend on Tauri; the shell receives events through `ServerConfig.hooks`
  (`on_settings`, `on_window_action`, `on_open_url`).
- **Sound-site preview window**: `open_browse_window` opens or reuses the `browse` window (see the sound sites part of §12).
- **Embedded front-end**: the service library embeds `apps/web/dist` with `rust-embed`, so the app doesn't depend on the
  working directory; `index.html` is served `no-cache` so an upgrade never shows a stale UI.

## 10. Phone remote

The phone **doesn't process audio**; it's only a remote control:

```
phone browser ──HTTP / LAN──▶ FerryMic on the computer ──commands──▶ audio engine ──▶ sound card / virtual cable
```

- **The main UI always listens on `127.0.0.1` only.** The phone remote is a **separate listener** (`0.0.0.0:main port + 1`,
  a random port if taken), started / gracefully stopped at runtime by `RemoteServer` — **no restart**; the phone button
  on the left rail toggles it.
- Both listeners share one axum `Router` (the same `AppState`); the phone side adds a `require_token` middleware:
  - the access code `remote_token` lives in settings; the address looks like `http://192.168.x.x:8322/?t=<code>`;
    after the first visit it's stored in the `sb_token` cookie, so later visits need no parameter; without a code
    a 401 page is returned.
  - "New access code" invalidates the old one immediately.
  - Desktop-only endpoints (`/api/remote`, `/api/settings`, `/api/pick-folder`, `/api/library/location`,
    `/api/library/open`, `/api/output`, `/api/open-url`, `/api/window`) allow only GET from phones.
  - The close prompt is for the PC window only: `/api/live` always returns `close_prompt: false` to phone requests.
  - `require_token` marks phone requests with `FromPhone`: phones may toggle the monitor and set its volume
    (`POST /api/monitor`), but the handler refuses (403) a monitor device change based on that mark.
- The panel shows the address and a QR code (SVG from the `qrcode` crate, `GET /api/remote/qr.svg`);
  the computer's LAN IP is found by querying the routing table, without sending packets.
- `SOUNDBRIDGE_LAN=1/0` forces it on / off (overrides the setting).
- This sidesteps the platform limit that mobile apps can't inject a system-wide microphone.

## 11. Output, monitor and virtual microphone

### 11.1 Output line (main output)

- Priority: `SOUNDBRIDGE_OUTPUT_DEVICE` (substring match) > the `output_device` setting > system default.
- `CpalBackend::resolve_output` resolves the device and takes its native config (shared-mode rate / channels);
  the soundboard and the mic both run at that rate.
- **Classification** (`describe_output`): if an input device forms a feedback loop with it (its other end, `game_mic_for`),
  or its name is virtual → `Virtual`, together with the mic other apps should pick (e.g. `CABLE Input` → `CABLE Output`);
  otherwise `Speaker`. The rightmost icon in the bottom bar tells the mic to pick in other apps, or turns into a yellow warning when outputting to a speaker.
- **Switching at runtime** (`POST /api/output`): the output thread receives `OutputRequest::Select`, drops the old stream,
  builds a new `SoundBoard`, runs `reinstall_all` for the decoded sounds, then opens the new device; on failure it
  reopens the previous device and returns 400. On success it notifies the mic thread (`OutputChanged`) and the
  monitor thread (`MainChanged`) and remembers the choice.

### 11.2 Monitor (`monitor.rs`)

The output goes to a virtual cable, so you can't hear the effects yourself. The monitor plays an **effects-only** copy on headphones / speakers:

- In the output callback, after `SoundBoard::fill` and before the mic is added, `MonitorTap` writes the current buffer
  (downmixed / expanded to stereo) into an SPSC ring; the monitor thread's second output stream reads it with `MonitorReader`.
- **No own microphone**: hearing a delayed echo of your own voice is unpleasant.
- **Different sample rates**: linear interpolation by `main rate / monitor rate`; output starts after about 20 ms,
  and any backlog over 60 ms is dropped, so drift between the two clocks doesn't keep adding latency.
- **Skipped when pointless**: if the main output is itself a speaker, or the monitor device is the main output device,
  the status shows why and nothing is opened.
- Settings: switch (on by default), device (default = system default playback device), volume. Clicking the headphones button at the
  bottom toggles the monitor; while it's on, hovering slides out the monitor volume bar on its right (the space is
  reserved while on and the bar is revealed with `clip-path`, so the button never moves from under the cursor; on
  devices without hover, such as phones, it stays visible).
- **How the volumes relate**: the volume on a card affects what others and you hear; the monitor volume only scales the monitor path, so only you hear the change.
- Ring hand-over works like the mic's: the new ring goes to the output callback over a channel, the old ring returns to the control side to be freed.

### 11.3 Linux automatic virtual microphone (`virtual_mic.rs`)

```
FerryMic output ──▶ virtual output soundbridge_mix (module-null-sink, description FerryMic-Mix)
                        └─ monitor ──▶ virtual mic soundbridge_mic (module-remap-source, description FerryMic-Microphone)
                                             └─▶ picked as the microphone in other apps
```

- Conditions: Linux, no output device specified (neither env var nor setting), and `auto_virtual_mic` true (default).
- Created with `pactl load-module` (works with PipeWire's pipewire-pulse and with PulseAudio) and removed with
  `unload-module` when the service ends; at start-up `pactl list short modules` is checked first, and modules left
  behind by a crash are taken over.
- cpal uses ALSA on Linux and can't select PulseAudio devices by name, so `PULSE_SINK` / `PIPEWIRE_NODE` =
  `soundbridge_mix` are set **before** any stream opens, sending this process's playback into the virtual output (playback only).
- Failure (e.g. no `pactl`) isn't fatal: Settings shows the reason and sounds play on the default device.
- The functions parsing `pactl` output are platform-independent and unit-tested on Windows too; the Linux build compiles
  and passes tests in CI, but the whole flow hasn't been tried on a real Linux desktop yet.
- Windows / macOS have no built-in virtual microphone API and still need a third-party cable such as VB-Cable / BlackHole (see ADR-0004).

## 12. UI (`apps/web`)

- Framework-free TypeScript: `api.ts` (types + requests), `icons.ts` (hand-drawn SVG icons), `i18n.ts` (translations),
  `sites.ts` (built-in sound sites), `main.ts` (UI logic).
- **Layout**: a slim left rail (library, sound sites, phone remote switch — its waves light up when on — help, settings),
  a folder sidebar (folder tree + new / rescan + library location), the sound card grid in the middle (search / stop all /
  import), and a bottom bar: your mic (mute / level / volume), the monitor button (headphones; when on, hovering slides
  out its volume bar), and the output target icon (hover for the mic to pick; click for Settings → "Audio chain").
- **Wording**: the UI doesn't name specific apps (games, Discord, …); it says "output line" and "other apps".
- On narrow screens (≤720 px, phone remote) both sidebars hide, folders become horizontal chips at the top, and the settings button moves to the toolbar.
- **Folders**: selecting a folder shows its sounds (including subfolders); the "⋯" menu creates subfolders / renames / deletes;
  cards can be dragged onto folders; files dropped from the system go into the folder they're dropped on, otherwise the current folder.
- **Cards**: the top shows the play button, name, and duration / hotkey / folder (one line); the middle is the seek bar
  (always takes space but only shows while playing, so cards stay the same height; drag to seek); the bottom has quick controls:
  - loop / once: a music-player style double-arrow icon, click to toggle;
  - exclusive / overlap: the same kind of icon button (dot / two overlapping circles), click to toggle;
  - neither state of these two switches is "better", so they're transparent and borderless, same colour in both states — only the icon changes;
  - volume: opens a vertical slider (saved when you stop dragging);
  - speed: text only; click for the next preset, right-click for the previous one.
  "⋯" opens the full editor (name = file name, folder, loop, exclusive, volume, speed, hotkey, delete).
- **Settings**: appearance (incl. "app icon follows the accent" and language), library location, defaults for new sounds,
  audio chain (chain diagram, your mic, output line, monitor, Linux virtual mic), window (close button behaviour), help.
  Items that only make sense in the desktop app (icon, window) carry `.app-only` and are hidden in browsers / on phones (`is_app` from `/api/app`).
- **Sound sites** (`sites.ts`): the globe button on the left switches to the "Sound sites" view (the folder sidebar and grid hide, the bottom bar stays).
  - The built-in list lives in the front-end (name, address, one-line description, licence summary; description and licence
    go through `t()`); user-added sites are stored in the `custom_sites` setting (at most 50, names 1–60 characters, http(s)
    only). Adding asks for the address first (`https://` is added if no scheme) and then the name (host by default).
  - Card buttons: "Preview" (eye icon) and "Open in browser" (globe icon). The search box at the top right filters by name, address, description and licence.
  - Opening: the desktop app calls `POST /api/open-url` → the `on_open_url` hook → the shell opens (or reuses) the preview
    window labelled `browse`; with `external` the system browser is used (Windows `rundll32 url.dll,FileProtocolHandler`).
    The browser version / phones just open a new tab. `/api/open-url` is desktop-only, so phones can't open windows on the computer.
  - **Downloads go straight into the library**: the preview window's `on_download` hook checks the file name; audio files
    are redirected to the library's "下载" folder (`Downloads` in the English UI). If that folder already has a file of the
    same name, the download is cancelled (no more copies) and the "failure" caused by cancelling is not reported.
    Other files go to the system downloads folder as usual.
  - **Preview window = custom title bar + site**: the system title bar can't host custom buttons, so the preview window has
    no system frame (`decorations(false)`) and holds two webviews (Tauri multi-webview, `unstable` feature): a local page
    `apps/app/loading/titlebar.html` on top (site name, current address, globe button, minimize / maximize / close) and
    the site below. `layout_browse_window` re-lays them out when the window resizes.
    - Only the title bar may drag and use these window buttons (`apps/app/capabilities/preview-titlebar.json`, scoped to
      `browse-bar`); the site has no Tauri permissions at all. Dragging, double-click to maximize, Aero Snap and resizing
      from the edges work; the Win11 layout menu on hovering the maximize button doesn't appear.
    - The globe button navigates to the reserved address `https://ferrymic-open.invalid/`; the title bar's `on_navigation`
      intercepts it, reads the site's current URL and opens it in the system browser. `on_page_load` updates the address
      in the title bar when the site navigates.
  - **Download notices** (`apps/app/preview/preview.js`, injected into the site, in a Shadow DOM unaffected by the site's
    styles): on download start / finish / skip / failure the shell calls `__ferryToast` via `eval` to show a notice at the
    top right of the site area; after a finished download the main window is told (`ferrymic:rescan` event) to rescan at
    once. No per-site parsing is needed — the site's own download button is the "one-click import".
- **Languages** (`i18n.ts`): Chinese is the source language; English is looked up by the Chinese text (`t('中文 {param}', {param})`).
  - Static HTML text (text nodes and `title` / `placeholder` / `aria-label` / `alt`) is replaced as a whole by
    `translateDom`, with originals kept in a `WeakMap` and restored when switching back; formatted blocks use
    `data-i18n-html="key"` and are swapped wholesale.
  - Missing entries show the Chinese text, so a gap never breaks anything. User data such as device and folder names isn't translated.
  - The preference is stored in the server settings; `/api/app` returns the resolved `language_effective` (`system` is
    decided by the backend from the OS language: Chinese systems get Chinese, everything else English), and the UI follows
    it; `localStorage` remembers the last language to avoid a flash at start-up.
  - After switching, static text changes at once and dynamic content (folders, cards, bottom bar, device lists) re-renders.
  - Backend (`i18n.rs`): an atomic holds the current language; `tr!("中", "en")` / `trf!("中 {x}", "en {x}")` produce
    user-facing messages and errors; logs stay bilingual. The desktop app rebuilds the tray menu on the `on_settings` hook.
  - Adding a language: add a dictionary in `i18n.ts`, a branch in the `i18n.rs` macros, and an entry in `LANGUAGES`.
- **Phone remote panel**: switch, QR code, address, new access code; clicking the rail button while it's off turns it on directly.
- **Appearance**: dark / light / follow system; 7 accent presets + a custom colour picker. A script derives `--accent`,
  `--accent-2` (a second gradient colour, hue +35°), `--accent-soft` / `--accent-glow` / `--accent-line` (different
  opacities) from the accent and writes them to `<html>`; they're also kept in `localStorage` and applied before the page
  renders on the next start to avoid a flash. The settings themselves live in the server's `settings.json` (the app's port
  may change, so `localStorage` isn't reliable).
- **Help**: Settings contains "Basics" and "Connecting other apps on each platform", with the current platform marked;
  the "?" button on the rail opens it and scrolls there.
- **Live state**: `/api/live` is polled every 100 ms (playing sounds and progress, mic level, whether to show the close
  prompt) and only the affected elements update, without re-rendering the grid; `/api/mic` (device status) is polled every
  1 s. Playing state comes from `PlayingSet`, written by the output callback, so sounds triggered by hotkeys light up and show progress too.
- **Disk changes**: the library is rescanned when the window regains focus (at most once every 2 s); there's also a "Rescan" button on the left.

## 13. REST API

| Method | Path | Description |
|---|---|---|
| GET | `/api/app` | Version, platform, data directory, library location, output device, port, appearance, phone remote, `is_app`, `tint_app_icon`, `close_action`, `language` / `language_effective`, `sound_defaults`, `custom_sites`, virtual mic status |
| POST | `/api/settings` | Update settings `{theme_mode?, accent?, auto_virtual_mic?, tint_app_icon?, close_action?, language?, sound_defaults?, custom_sites?}` (calls the `on_settings` hook afterwards) |
| POST | `/api/window` | Answer the close prompt `{action: "tray" \| "quit" \| "cancel", remember}` |
| POST | `/api/open-url` | Open an address `{url, title?, external}`; http(s) only; the app's preview window by default |
| POST | `/api/pick-folder` | Show the system "choose folder" dialog on the computer (`rfd`); returns `{path}` |
| GET | `/api/live` | `{playing: [{id, progress}], mic_level, mic_connected, close_prompt}` |
| GET | `/api/devices` | `{outputs: [{name, is_default, kind, game_mic}], inputs: [{name, is_default, virtual}], current}` |
| POST | `/api/output` | Switch the main output `{device: "name" \| null}`; returns the new output info; 400 and the previous device restored on failure |
| GET / POST | `/api/monitor` | Monitor status / settings `{enabled?, device?, gain_db?}` (`device: null` = system default) |
| GET / POST | `/api/remote` | Phone remote `{enabled, url, port, error}` / switch `{enabled?, regenerate?}` |
| GET | `/api/remote/qr.svg` | QR code of the current phone address (SVG) |
| GET | `/api/mic` | Mic status: selection, device in use, sample rate, connected, note, volume, muted, level |
| POST | `/api/mic` | Set the mic `{gain_db?, muted?}` (the volume is remembered) |
| POST | `/api/mic/device` | Switch the mic `{device: "name" \| null}`; `null` = auto |
| GET | `/api/library` | `{root, folders: [relative paths], sounds: [...]}` |
| POST | `/api/library/rescan` | Rescan the disk and sync loading / hotkeys |
| POST | `/api/library/location` | Change the library location `{path, move_contents}` |
| POST | `/api/library/open` | Open the library in the system file manager |
| POST | `/api/folders` | Create a folder `{path}` |
| PATCH | `/api/folders` | Rename / move `{from, to}` (`to` is the new full relative path) |
| DELETE | `/api/folders` | Delete a folder without sounds `{path}` (to the recycle bin) |
| GET | `/api/sounds` | List sounds |
| POST | `/api/sounds/import` | Upload (multipart: `file`, optional `folder`; up to 200 MB) |
| PATCH | `/api/sounds/:id` | Edit `{name?, folder?, looping?, exclusive?, gain_db?, speed?, hotkey?}`; `name` renames the file, `folder` moves it, `hotkey: null` clears, omitted fields are unchanged |
| DELETE | `/api/sounds/:id` | Delete (the file goes to the recycle bin) |
| POST | `/api/sounds/:id/trigger` · `/stop` | Trigger (loading first if needed) / stop |
| POST | `/api/sounds/:id/seek` | Seek `{position: 0..1}` |
| POST | `/api/stop-all` | Stop everything |

## 14. Data model (front-end JSON)

```jsonc
// A sound (an item of sounds in GET /api/library)
{
  "id": 1,
  "name": "Applause",           // file name without extension
  "folder": "Fun/Reactions",    // relative to the library root; "" for the root
  "file_name": "Applause.mp3",
  "looping": false,             // loop (false = once)
  "exclusive": false,           // exclusive (false = overlap)
  "gain_db": 0.0,
  "speed": 1.0,
  "hotkey": "Ctrl+Alt+KeyQ",   // optional; null = none
  "duration_secs": 2.4,         // known once loaded
  "loaded": true,               // decoded into memory
  "error": null                 // load failure reason
}

// Microphone (GET /api/mic)
{
  "selection": null,            // user choice; null = auto (prefer a hardware mic)
  "device": "Microphone (WO Mic Device)",
  "sample_rate": 48000,
  "connected": true,
  "note": null,                 // reason when not connected, e.g. "Waiting for the microphone…"
  "gain_db": 0.0,
  "muted": false,
  "level": 0.12                 // 0..1, after gain, before mute
}

// Monitor (GET /api/monitor)
{
  "enabled": true,
  "selection": null,            // null = default playback device
  "device": "Headphones (AirPods)", // the device actually opened
  "active": true,               // monitoring right now
  "note": null,                 // why it isn't active, e.g. "The output is already speakers / headphones; no monitor needed"
  "gain_db": 0.0
}
```
