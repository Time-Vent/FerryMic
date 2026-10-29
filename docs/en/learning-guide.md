# FerryMic Learning Guide: from "knows basic syntax" to "can read, test and change it"

**English** | [简体中文](../learning-guide.md)

> For computer science students who know basic programming syntax (variables, functions, loops, classes / structs).
> After reading this guide you should be able to explain what the whole project does, know what every folder and key
> file is for, find a starting point for reading the code, run and test it yourself, and make small improvements.
>
> Companion docs: [Code walkthrough](code-walkthrough.md) (a module-by-module "dictionary") and
> [Soundboard architecture](soundboard-architecture.md) (design details). This guide is the "map + route";
> those two are the "encyclopedia". Read this first, then look things up there as needed.

## Contents

0. [How to use this guide](#0-how-to-use-this-guide)
1. [Use it first: what the project actually does](#1-use-it-first-what-the-project-actually-does)
2. [Background you need (only what's used)](#2-background-you-need-only-whats-used)
3. [Toolchain: what each command does](#3-toolchain-what-each-command-does)
4. [Folder map: what every file is](#4-folder-map-what-every-file-is)
5. [Follow one click all the way through](#5-follow-one-click-all-the-way-through)
6. [Why it's designed this way: five core ideas](#6-why-its-designed-this-way-five-core-ideas)
7. [Recommended reading order: where to start](#7-recommended-reading-order-where-to-start)
8. [Running, debugging and testing](#8-running-debugging-and-testing)
9. [Exercises: hands-on changes, easy to hard](#9-exercises-hands-on-changes-easy-to-hard)
10. [Common pitfalls](#10-common-pitfalls)
11. [Glossary](#11-glossary)

---

## 0. How to use this guide

Work through four stages, each with a clear "done when":

| Stage | Sections | Rough time | Done when |
|---|---|---|---|
| ① See the whole | 1, 2, 4 | half a day | you can draw the flow "sound from the mic to other apps" and say what each top-level folder is for |
| ② Get it running | 3, 8 | half a day | you can build the app from source, run the tests and debug with the browser version |
| ③ Read the main line | 5, 6, 7 | 2–3 days | you can follow one click from the UI code all the way to the line that makes the sound |
| ④ Change things | 9, 10 | ongoing | you've done 2–3 exercises and all tests pass |

**A mindset**: the project has over twenty thousand lines of code, and you **don't need to understand all of it**.
Professionals facing an unfamiliar project also "grab the main line first and look things up as needed".

---

## 1. Use it first: what the project actually does

### 1.1 In one sentence

FerryMic is a **desktop "mic + soundboard"**: it mixes your microphone with sound effects and sends the result into a
"virtual microphone"; other apps (voice chat, meetings, streaming) use it as their microphone, so the other side hears
both your voice and the effects.

### 1.2 The sound's journey (the single most important diagram)

```
 your real mic ──────▶ ┌──────────────── FerryMic ────────────────┐
                       │                                          │
 sound files (mp3/wav) ─▶ decode to numbers ─▶ mix (add) ─▶ limit ─┼──▶ virtual cable CABLE Input
                       │                    │                     │         │
                       │                    └─▶ effects-only copy ─┼──▶ your headphones (monitor)
                       └──────────────────────────────────────────┘         │
                                                                              ▼
                                                    other apps use CABLE Output as their mic
```

A **virtual audio cable** (e.g. VB-Cable) is a fake sound card: sound put into its "input end" (CABLE Input) can be
recorded from its "output end" (CABLE Output) as if it were a microphone. Windows can't do this by itself, so a
third-party one is needed; on Linux, FerryMic creates one with system commands.

### 1.3 What the program looks like (three layers)

```
┌─────────────────────────────────────────────┐
│ UI (web tech: HTML + CSS + TypeScript)       │  apps/web
│   everything you see: cards, buttons, settings│
└───────────────▲─────────────────────────────┘
                │ HTTP requests (just like visiting a website)
┌───────────────┴─────────────────────────────┐
│ Backend service (Rust)                       │  apps/server
│   library, loading, hotkeys, mic, output, remote │
└───────────────▲─────────────────────────────┘
                │ function calls
┌───────────────┴─────────────────────────────┐
│ Audio core libraries (Rust)                  │  crates/*, adapters/*
│   decoding, varispeed, mixing, talking to the sound card │
└─────────────────────────────────────────────┘
Wrapped in a "desktop app shell" (Tauri):          apps/app
   provides the window and tray icon; the window shows the web page above
```

**Why a web page for the UI?** It's the easiest way to build a good-looking UI, and the same UI works directly on a
phone (the phone remote is just the phone's browser opening the same page).

---

## 2. Background you need (only what's used)

### 2.1 Ten digital-audio concepts

| Concept | In one sentence | What it looks like in code |
|---|---|---|
| **Sample** | Sound is air vibrating; the computer records the vibration's position at regular intervals, and each recorded number is a sample | an `f32` in -1.0 to 1.0 |
| **Sample rate** | How many times per second. 48000 Hz = 48,000 samples per second | `sample_rate: u32`, `SampleRate(48000)` |
| **Channels** | Mono is 1, stereo is 2 (left, right) | `channels` |
| **Frame** | All channels' samples at one moment. A stereo frame = 2 samples | `frames`; `samples = frames × channels` |
| **Interleaved** | Stereo is stored as `L R L R …` in the array | every `&[f32]` buffer is interleaved |
| **Mixing** | Adding several sounds **sample by sample** | `out[i] += sound[i] * gain` |
| **Gain / dB** | Volume. dB is logarithmic: +6 dB ≈ double, 0 dB = unchanged, -60 dB ≈ silence | `gain_db`, converted with `db_to_linear()` |
| **Clipping** | A sum can exceed 1.0, which distorts, so it's cut to [-1, 1] | `s.clamp(-1.0, 1.0)` |
| **Resampling** | The file is 44100 Hz, the sound card 48000 Hz; convert to a common rate | `rate_ratio`, linear interpolation |
| **Buffer + callback** | The sound card doesn't ask for one sample at a time; every few milliseconds it says "give me 480 frames" and calls a function you wrote (the callback) to fill them | `let callback = move \|data: &mut [f32]\| { ... }` |

**The key point**: the sound card calls the callback with a **deadline**. At 48000 Hz and 480 frames per call you have
**10 milliseconds** to fill the data; late means stutters / clicks. This rule shapes the whole project (see section 6).

### 2.2 Concurrency: why there are so many threads

A program can run several "lines of execution" (threads) at once. FerryMic has about ten:

- **The sound card callback thread**: only fills sound, must be fast.
- **The loader thread**: reads and decodes files (slow, can take hundreds of milliseconds).
- **The mic thread**, **the hotkey thread**, **the web server threads**…

Threads pass data with these tools, which you'll see again and again:

| Tool | Use | Analogy |
|---|---|---|
| **Channel** | one thread "sends a message", another "receives it" | passing notes |
| **Atomic** | a number that many threads can read and write at the same time safely, without locks | a number on a notice board |
| **Ring buffer (SPSC)** | one thread keeps writing sound, another keeps reading, looping end to end | a conveyor-belt sushi bar |
| Lock (Mutex) | only one thread may touch some data at a time | a bathroom door lock — **forbidden in the callback thread**, because waiting for it may exceed 10 ms |

### 2.3 Rust concepts in a nutshell

You only need to recognise these to match them up while reading:

```rust
// Result: a function that may fail returns Ok(value) or Err(error); a trailing ? means "return the error if it failed"
fn load(path: &str) -> Result<Sound, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // ...
}

// Option: may have no value, Some(value) or None
let device: Option<String> = None; // None means "use the system default"

// struct + impl: data + methods (like a class in other languages)
struct Player { /* ... */ }
impl Player {
    pub fn trigger(&self, id: SoundId) { /* ... */ }
}

// trait: an interface ("these methods must exist")
pub trait AudioBackend { fn open_output(/* ... */); }

// Arc<T>: share one thing between threads (reference counted); Mutex<T>: access under a lock
let shared = Arc::new(Mutex::new(Vec::new()));

// closure: an anonymous function; move means the variables it uses are "moved" into it
let callback = move |data: &mut [f32]| { /* fill the sound */ };

// async / await: asynchronous, doesn't hold a thread while waiting on the network. The web server (axum + tokio) uses it
async fn trigger_sound(/* ... */) -> StatusCode { /* ... */ }
```

- **Ownership and borrowing**: Rust guarantees at compile time that the same data isn't modified by two threads at
  once. When something doesn't compile, the error usually tells you how to fix it — **reading the compiler's errors
  carefully** is the fastest way to learn Rust.
- **crate**: a Rust "package"; one crate is one library or program. **workspace**: several crates managed together in one repository.
- **Macros**: names followed by `!`, like `println!` and `format!`; this project defines its own `tr!` / `trf!` (Chinese / English messages).

### 2.4 Front-end concepts in a nutshell

- **HTML** defines which elements exist (buttons, cards), **CSS** how they look, and **TypeScript** (JavaScript with
  types) what happens when you click.
- `fetch('/api/library')`: sends an HTTP request to the backend and gets back **JSON** (a text data format, `{"id": 1, "name": "Applause"}`).
- **Polling**: asking the backend "what's playing now?" every 100 milliseconds and updating the progress bars from the answer.
- The project **doesn't use a framework like React / Vue**; it manipulates page elements (the DOM) directly, so the code is more straightforward and good for learning.

### 2.5 What an HTTP API is

The backend exposes a set of "address + action" pairs, for example:

```
POST /api/sounds/3/trigger   → play sound 3
GET  /api/library            → return the sound list (JSON)
```

The UI, the phone, and even you with a `curl` command all control FerryMic through these addresses. **The full list
is in the "REST API" section of the README.**

---

## 3. Toolchain: what each command does

### 3.1 What to install

| Tool | Purpose | Install |
|---|---|---|
| **Rust** (rustup, cargo) | compiles Rust code; cargo is Rust's "build + package manager + test runner" | rustup.rs |
| **MinGW-w64** (Windows) | the GNU toolchain this project uses on Windows | `winget install BrechtSanders.WinLibs.POSIX.UCRT` |
| **Node.js + pnpm** | builds the web UI; pnpm is a faster alternative to npm | `winget install OpenJS.NodeJS.LTS`, `npm install -g pnpm` |
| **Git** | version control | git-scm.com |
| VB-Cable (optional) | a virtual audio cable, needed to test "can other apps hear it" | vb-audio.com |

### 3.2 Common commands

Run these in the project root (`FerryMic/`):

| Command | What it does | When |
|---|---|---|
| `cargo build` | compiles the whole workspace (debug build: fast to build, unoptimised) | after changing Rust code, to see if it compiles |
| `cargo test` | runs all unit tests | always after changing code |
| `cargo test -p soundbridge-server` | tests one crate only | faster when you only changed the server |
| `cargo test exclusive` | runs only tests whose name contains `exclusive` | debugging one test |
| `cargo clippy --workspace --all-targets -- -D warnings` | lints (pickier than the compiler; warnings count as failures) | before committing |
| `cargo fmt --all` | formats the code automatically | before committing |
| `cargo run --bin soundbridge-server` | runs the browser version of the service | debugging the UI and API (see 8.2) |
| `cargo clean` | deletes the build cache `target/` | when you need disk space |

Run these in `apps/web/`:

| Command | What it does |
|---|---|
| `pnpm install` | installs front-end dependencies (into `node_modules/`) |
| `pnpm build` | compiles and bundles the TypeScript into `dist/` (the backend embeds this folder into the program) |
| `pnpm exec tsc --noEmit` | TypeScript type checking only |

To package the whole desktop app: `powershell -ExecutionPolicy Bypass -File .\build-app.ps1` (Windows) or `./build-app.sh` (Linux).

### 3.3 What GitHub does automatically

- **CI** (`.github/workflows/ci.yml`): on every push, runs formatting checks, clippy and tests on Windows and Linux machines in the cloud.
- **Release** (`.github/workflows/release.yml`): when a tag like `v0.1.0` is pushed, builds the installers and puts them in a draft release.

---

## 4. Folder map: what every file is

### 4.1 Top level

```
FerryMic/
├── Cargo.toml          ← workspace config: which crates exist, shared dependency versions, release optimisation options
├── Cargo.lock          ← [generated] pins the exact version of every dependency so everyone builds the same thing. Commit it, don't edit it by hand
├── rust-toolchain.toml ← use stable Rust + the Windows GNU target
├── rustfmt.toml / clippy.toml / deny.toml ← rules for formatting, linting and dependency licence checks
├── justfile            ← shortcuts for common commands (needs the just tool, optional)
├── build-app.ps1 / build-app.sh ← one-step packaging of the desktop app
├── start-cable.ps1 / start-speaker.ps1 ← quick launchers for the browser version (output to the virtual cable / speakers)
├── README.md / README.en.md ← project description (Chinese / English)
├── .gitignore          ← tells git which files not to commit
├── .github/workflows/  ← CI and automatic release config
├── crates/             ← audio core libraries (pure Rust, independent of the UI)
├── adapters/           ← implementations that talk to real sound cards
├── apps/               ← runnable programs
├── docs/               ← documentation
├── tests/fixtures/     ← audio material for tests
│
├── target/             ← [generated] Rust build output and cache, can be several GB. Not committed; delete any time (cargo clean)
└── dist-app/           ← [generated] the exe packaged by build-app. Not committed
```

### 4.2 `crates/`: the audio core (bottom to top by dependency)

| Crate | In one sentence | Files worth reading |
|---|---|---|
| `audio-types` | the lowest-level data types (sample rate, channels, gain, …), depends on no other crate | `src/lib.rs` |
| `platform-api` | defines what a "sound card backend" must provide (traits), with no implementation | `src/lib.rs` |
| `dsp` | audio processors: gain, limiter, EQ | `gain.rs`, `limiter.rs`, `biquad.rs` |
| `audio-graph` | chains several processors together | `src/lib.rs` |
| `audio-source` | reads audio files, decodes, **changes speed** (speed playback relies on it) | `varispeed.rs` (the core), `audio_file.rs` |
| `audio-engine` | the early engine (used by the command-line demo): executor roles, clock drift control | `executors.rs`, `clock.rs` |
| `application` | assembles the early engine (used by the command-line demo) | `app.rs` |
| **`soundboard`** | **the soundboard core: manages many sounds, play / stop / loop / exclusive, mixing**. The main product's most important pure logic | `src/lib.rs` (**recommended first read**) |

### 4.3 `adapters/audio-cpal/`: connecting to real sound cards

Implements the interfaces from `platform-api` with the open-source library **cpal** — WASAPI on Windows, ALSA on Linux.
Key functions: `resolve_output` (find a device by name), `build_output` (start sending sound to the card), `build_input`
(start receiving sound from the mic).

### 4.4 `apps/`: runnable programs

| Folder | What it is |
|---|---|
| `apps/server/` | **the backend service library + browser version program**. The main product's brain, see below |
| `apps/web/` | **the UI** (a web page) |
| `apps/app/` | **the desktop app shell** (Tauri): window, tray, close prompt, icon tint, sound-site preview window |
| `apps/desktop/` | the early command-line demo (`soundbridge --list`) |
| `apps/mobile/` | mobile placeholder, no code |

**`apps/server/src/` file by file** (the core of the main product; worth reading closely):

| File | Responsible for |
|---|---|
| `main.rs` | the browser version's entry, a dozen lines: read environment variables → call `run()` |
| `lib.rs` | **the start-up flow `run()`**: read settings → open the library → start the threads → start serving the web page |
| `api.rs` | **every HTTP endpoint**: each `/api/...` address maps to an `async fn`. Every button in the UI ends up here |
| `library.rs` | library = a folder on disk: scanning, importing, renaming, moving, deleting, and reading / writing `.soundbridge.json` |
| `loader.rs` | the loader thread: decodes sounds into memory in the background; `Player::trigger` is **the single entry for playing** |
| `audio.rs` | **the output thread and the sound card callback**: mixes every 10 ms and hands the result to the sound card |
| `mic.rs` | the mic thread: picks a mic automatically, reconnects, guards against feedback |
| `monitor.rs` | the monitor: sends the effects-only copy to the headphones |
| `hotkeys.rs` | global hotkeys |
| `remote.rs` | the phone remote: LAN listener, access code, QR code |
| `settings.rs` | saving and loading settings (`settings.json`) |
| `i18n.rs` | Chinese / English messages |
| `virtual_mic.rs` | creating the virtual microphone automatically on Linux |

**`apps/web/` file by file**:

| File | Responsible for |
|---|---|
| `index.html` | the page skeleton: the HTML of every dialog, button and setting |
| `src/main.ts` | **the UI logic** (the biggest file): rendering cards, handling clicks, polling state, the settings page |
| `src/api.ts` | TypeScript types and call functions for every backend endpoint (read it to see what front and back exchange) |
| `src/i18n.ts` | the Chinese → English dictionary |
| `src/icons.ts` | all icons (SVG) |
| `src/sites.ts` | the built-in list of sound sites |
| `src/style.css` | all styles, dark / light themes |
| `dist/` | [generated, but committed] the output of `pnpm build`, embedded into the backend program |
| `node_modules/` | [generated] front-end dependencies, not committed |
| `package.json` / `pnpm-lock.yaml` / `pnpm-workspace.yaml` | front-end dependency list / pinned versions / pnpm settings |
| `vite.config.ts` / `tsconfig.json` | config for the bundler and TypeScript |

**`apps/app/`**:

| File | Responsible for |
|---|---|
| `src/main.rs` | all of the desktop shell's logic: starting the backend, windows, tray, close prompt, icon tint, preview window, downloads into the library |
| `tauri.conf.json` | program name, icons, packaging settings |
| `loading/index.html` | the error page shown if the backend fails to start |
| `loading/titlebar.html` | the preview window's custom title bar |
| `preview/preview.js` | the "download notice" script injected into preview pages |
| `capabilities/` | Tauri permission config |
| `icons/` | program icons |
| `gen/schemas/` | [generated] Tauri config schemas |
| `build.rs` | a small script run before compiling (Tauri uses it to read the config and embed the icon) |

### 4.5 Files created at runtime (not in the project)

| Location | Contents |
|---|---|
| `%APPDATA%\FerryMic\settings.json` | all settings |
| `%APPDATA%\FerryMic\sounds\` | the default library |
| `<library>\.soundbridge.json` | each sound's settings (loop, volume, hotkey, …) |

---

## 5. Follow one click all the way through

This is the most effective way to understand the project: **pick one user action and follow it through the code**.
Here we follow "click a card → hear the sound".

### Stop 1: the UI (TypeScript)

In `apps/web/src/main.ts`, inside the function `renderPad`:

```ts
const fire = () => {
  if (!s.error) api.trigger(s.id).catch((e) => toast(errorText(e), 'error'));
};
pad.addEventListener('click', fire);
```

Clicking the card → calls `api.trigger(3)`. In `apps/web/src/api.ts` you can see that this is just:

```ts
trigger: (id: number) => request<void>(`/api/sounds/${id}/trigger`, { method: 'POST' }),
```

i.e. it sends `POST /api/sounds/3/trigger` to the backend.

### Stop 2: the HTTP endpoint (Rust, axum)

In `apps/server/src/api.rs`, the route table has this line:

```rust
.route("/sounds/:id/trigger", post(trigger_sound))
```

meaning "when a POST arrives at this address, call `trigger_sound`":

```rust
async fn trigger_sound(State(state): State<AppState>, Path(id): Path<SoundId>) -> StatusCode {
    state.player.trigger(id);
    StatusCode::OK
}
```

`Path(id)` extracts the `3` from the address automatically; `State(state)` is the "global state" shared by every
endpoint (library, player, settings, …).

### Stop 3: the player (the loader thread)

`apps/server/src/loader.rs`:

```rust
pub fn trigger(&self, id: SoundId) {
    if self.is_loaded(id) {
        let _ = self.cmd_tx.send(BoardCommand::Trigger { sound_id: id });
    } else {
        let _ = self.urgent_tx.send(Job::LoadAndTrigger(id));
    }
}
```

- Already decoded in memory: send the sound card thread a "play sound 3" command **through a channel**.
- Not decoded yet: hand it to the loader thread to decode first (slow work must not run on the sound card thread), then send the command.

### Stop 4: the sound card callback (every 10 milliseconds)

In `apps/server/src/audio.rs`, `let callback = move |data: &mut [f32]| { ... }`:

```rust
// 1. Take up to 32 commands and apply them (such as the "play sound 3" just sent)
for _ in 0..32 {
    match cmd_rx.try_recv() { Ok(cmd) => board.apply_command(cmd), Err(_) => break }
}
// 2. Mix every playing sound into data
board.fill(data, frames, out_ch);
playing.publish(&board);             // tell the UI what's playing and how far along
// 3. The effects-only copy for the monitor
monitor_tap.tap(data, frames, out_ch);
// 4. Add the mic
mic_reader.mix_into(data, frames, out_ch, &mic);
// 5. Limit
for s in data.iter_mut() { *s = s.clamp(-1.0, 1.0); }
```

Once `data` is filled, cpal hands it to the sound card (the virtual cable), and other apps "hear" it.

### Stop 5: the mixing core

`crates/soundboard/src/lib.rs`:

- `apply_command` receives `Trigger` → calls `trigger`: handles the play mode (pressing a looping sound again stops it,
  exclusive clears the others first), then creates a new "playing instance".
- `fill`: goes through every instance; each takes sound from its own position onwards (stepping by speed and the
  sample-rate ratio), multiplies by its volume and **adds** it to the output.

### Stop 6: how the UI knows it's playing

- `playing.publish(&board)` in the callback writes "sound 3, at 42%" into a set of **atomics** (no lock).
- The UI requests `GET /api/live` every 100 milliseconds (`startPolling` in `main.ts`) and updates the progress bar from the result (`applyPlaying`).

**You've now walked the whole main line.** Follow two more actions the same way and you've basically got the project:

- "Drag a file into the window to import it": `importFiles` in `main.ts` → `POST /api/sounds/import` → `import` in `library.rs`.
- "How the mic's sound gets in": `open` in `mic.rs` → `MicCapture::push` (the mic callback writes a ring buffer) →
  `MicReader::mix_into` in `audio.rs` (the sound card callback reads from the ring).

---

## 6. Why it's designed this way: five core ideas

### 6.1 Real-time safety: four "nevers" in the callback

The sound card callback must finish before its deadline, so it **never allocates memory, never locks, never reads or
writes files, and never panics**. This leads directly to:

- decoding on the loader thread (reading and decoding files are slow);
- the callback talking to other threads only through **channels, atomics and ring buffers**;
- sound PCM shared through `Arc`, so playing only bumps a reference count instead of copying.

**A check while reading**: whenever code runs in the callback, check it against the four "nevers".

### 6.2 One thread per kind of slow work

The loader thread, mic thread, monitor thread, hotkey thread… each handles its own job and talks by "request-reply"
(e.g. `MicRequest::Select { device, reply }`: the request carries a reply channel, and the result is sent back
through it). That way no slow operation blocks anyone else.

### 6.3 Front and back separated: the UI is just a web page

The backend only provides HTTP endpoints and doesn't care what the UI looks like. Benefit: the desktop window, the
browser and the phone use **the same UI and the same endpoints**; you can also control it directly with `curl` or scripts.

### 6.4 The disk is the database

The library is simply a folder, subfolders are categories, and each sound's settings live in `.soundbridge.json`.
There's no database: users manage files in the file manager, and the whole folder can be copied to another computer.

### 6.5 Testable logic separate from platform code

`crates/soundboard` is pure logic: give it a fake output buffer and you can test "is there sound after a trigger"
**without a real sound card**. That's why most of the project's tests run on cloud CI machines with no sound card.
When you write new features, try to split them the same way: the "computing" part apart from the "talking to hardware" part.

---

## 7. Recommended reading order: where to start

| Order | What to read | Why start here | Focus on |
|---|---|---|---|
| 1 | `crates/soundboard/src/lib.rs` | pure logic, no hardware, has tests — the easiest to understand | `PlayMode`, `apply_command`, `trigger`, `fill`; **read the tests at the end first** — tests are usage examples |
| 2 | `crates/audio-source/src/varispeed.rs` | understand "speed playback" and "linear interpolation" | `fill`, `interp`, `progress` |
| 3 | `apps/web/src/api.ts` | one file shows all the data front and back exchange | the `interface`s and the `api` object |
| 4 | `apps/server/src/api.rs` | how the backend handles each endpoint | the route table (`fn router`) → pick a few handlers |
| 5 | `run()` in `apps/server/src/lib.rs` | what the program does at start-up, in order | step by step against [Code walkthrough §12](code-walkthrough.md#12-server-soundboard-service-library) |
| 6 | `apps/server/src/audio.rs` | what a real-time callback looks like | the `callback` closure, `PlayingSet` |
| 7 | `apps/server/src/loader.rs`, `library.rs` | loading and the library | `Player::trigger`, `Worker::run`, `scan` |
| 8 | `apps/web/src/main.ts` | the UI logic (long; read selectively) | `renderPad`, `renderControls`, `startPolling` |
| 9 | the rest, by interest | `mic.rs` (device handling), `remote.rs` (security), `apps/app/src/main.rs` (desktop shell) | |

**Tips for reading code**:

- Install the **rust-analyzer** extension in VS Code: Ctrl-click a function name to jump to its definition, hover to
  see types. It's the most important tool for reading Rust.
- When a function is unclear, read its **comment** first (every function in this project has Chinese and English
  comments) and then **where it's called from**.
- Search the whole project (Ctrl+Shift+F) for a piece of text you see in the UI to find the front-end code, then follow `api.xxx` to the backend.

---

## 8. Running, debugging and testing

### 8.1 The rule for safe testing: don't let the computer suddenly make noise

It's easy to trigger sounds by accident while debugging audio code. Recommended:

- output to the virtual cable (`SOUNDBRIDGE_OUTPUT_DEVICE=CABLE Input`), not the speakers;
- start with the mic muted (`SOUNDBRIDGE_MIC_MUTED=1`) so speakers and mic can't howl;
- use a separate data directory (`SOUNDBRIDGE_DATA_DIR=some temp folder`) so your everyday settings and library stay untouched.

### 8.2 Debugging with the browser version (recommended)

The browser version is the same code as the desktop app, but much easier to debug:

```powershell
cd apps\web; pnpm install; pnpm build; cd ..\..
$env:SOUNDBRIDGE_DATA_DIR = "$env:TEMP\ferrymic-dev"
$env:SOUNDBRIDGE_OUTPUT_DEVICE = "CABLE Input"
$env:SOUNDBRIDGE_MIC_MUTED = "1"
$env:SOUNDBRIDGE_PORT = "8341"
cargo run --bin soundbridge-server
```

Then open `http://127.0.0.1:8341` in a browser and press **F12** for the developer tools:

- **Console**: front-end errors;
- **Network**: which request each button sends and what the backend returns — the most direct way to understand how front and back interact.

After changing front-end code: run `pnpm build` again and refresh the browser (the debug backend reads the `dist/`
folder directly). After changing Rust code: stop the service and `cargo run` again.

### 8.3 Logs

The backend logs with `log::info!` / `log::warn!` and shows the info level by default. For more detail:

```powershell
$env:RUST_LOG = "debug"; cargo run --bin soundbridge-server
```

### 8.4 Calling endpoints directly with curl

You can test the backend without the UI:

```bash
curl http://127.0.0.1:8341/api/library
```

```bash
curl -X POST http://127.0.0.1:8341/api/sounds/1/trigger
```

### 8.5 Running and writing tests

```bash
cargo test --workspace
```

Tests live at the end of each file in `#[cfg(test)] mod tests { ... }`. A real example (`crates/soundboard/src/lib.rs`):

```rust
#[test]
fn trigger_real_sound_produces_audio() {
    let dir = temp_dir("soundboard-test");
    let wav = make_test_wav(&dir, "sine.wav", 48_000, 4800); // generate a sine-wave file on the spot

    let mut board = SoundBoard::new(4800, 48_000);
    add(&mut board, 1, &wav, TriggerMode::OneShot);

    board.apply_command(BoardCommand::Trigger { sound_id: 1 });
    assert_eq!(board.instance_count(), 1);                     // assert: one is playing

    let mut out = vec![0.0f32; 512];
    board.fill(&mut out, 512, 1);                             // mix once
    let peak = out.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
    assert!(peak > 0.01, "expected sound, got peak={peak}");   // assert: there really is sound
}
```

A test always has three steps: **arrange (make data) → act (call the function) → assert (check the result)**. The
fastest way to write a new test is to copy a similar one and change it.

### 8.6 Checklist before committing

```bash
cargo fmt --all
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

If you changed the front-end, also run `pnpm exec tsc --noEmit` and `pnpm build` in `apps/web`. Only when all of
these pass will CI be green after you push to GitHub.

---

## 9. Exercises: hands-on changes, easy to hard

Each exercise lists **the files involved** and **hints**. Run the 8.6 checks after each one.

### ★ Beginner (one or two files)

1. **Change a piece of UI text and add its English translation**
   - Files: `apps/web/index.html`, `apps/web/src/i18n.ts`
   - Hint: the Chinese text is the dictionary "key"; if you change the Chinese, change the key in `i18n.ts` too. Switch the language in Settings to check.

2. **Add a speed preset (e.g. 3.0×)**
   - File: `SPEED_PRESETS` in `apps/web/src/main.ts`
   - Hint: the backend limits the speed; find what `speed` is clamped to in `api.rs` and `audio-types`, change it too if needed, and think about why it's limited.

3. **Add a test for `SoundDefaults::clamped`**
   - File: `apps/server/src/library.rs`
   - Hint: copy the existing `defaults_are_clamped` and test edge values such as negative infinity and very large speeds.

### ★★ Intermediate (front and back)

4. **Add an endpoint `GET /api/stats` returning the total number of sounds and how many are loaded**
   - Files: `apps/server/src/api.rs` (route + handler), `apps/web/src/api.ts` (type + call)
   - Hint: model it on `get_live`; `state.library.entries()` gives all sounds, `state.player.is_loaded(id)` tells whether one is loaded. Test with curl.

5. **Show these stats under "About" in Settings**
   - Files: `index.html`, `main.ts`
   - Hint: call the new endpoint in `openSettings` and write the result into a new element; remember the translation.

6. **Friendlier hotkey conflict message**: when a hotkey is taken, say which sound has it (by name, not number)
   - Files: `apps/server/src/hotkeys.rs`, `api.rs`
   - Hint: the error message currently contains `#{other}`; find a way to turn the id into a name in the API layer.

### ★★★ Challenge (real-time audio)

7. **Fade out on stop**: instead of cutting off with a "click", ramp the volume to 0 over 50 milliseconds
   - File: `crates/soundboard/src/lib.rs`
   - Hint: give `Instance` a "fade-out frames remaining" field; the `Stop` command no longer removes the instance
     immediately but starts the fade; in `fill`, multiply by a decreasing factor frame by frame and remove the instance
     when it reaches 0. **No allocation in the callback.** Write the test first: the first few samples after stopping
     aren't 0, and after 50 ms they're all 0.

8. **Master volume**: one slider controls the total volume of all effects (not the mic)
   - Files: `audio.rs` (multiply by a factor after `board.fill` in the callback), `api.rs`, `settings.rs`, the front-end
   - Hint: store the factor as the bit pattern of an f32 in an `AtomicU32` (see `gain_db` in `MicControl`); the callback only reads it, no lock.

9. **Level meter**: show the live volume of the effects mix in the bottom bar (like the mic level meter)
   - Hint: see the "fast attack, slow release" in `MicControl::update_level`; compute the peak after `board.fill` in the callback and pass it to the front-end through `/api/live`.

---

## 10. Common pitfalls

| Symptom | Cause and fix |
|---|---|
| Packaging fails with "file in use" | a FerryMic is still running (maybe minimized to the tray); quit it and package again |
| Double-clicking the new exe opens the old window | single-instance: while one is running, launching again only wakes the old one. Quit it fully first |
| `cargo build` is slow and `target/` is several GB | normal; the first build compiles hundreds of dependencies. Later builds only compile what changed |
| Changed the front-end but the desktop app looks the same | release builds **compile the front-end into the exe**; run `pnpm build` and repackage. Use the browser version while debugging |
| Chinese text turns into garbage after editing files in PowerShell | Windows PowerShell 5.1 reads and writes files as GBK by default. Edit source files in an editor, not with `Get-Content` / `Set-Content` |
| Hotkeys don't work on Linux | Wayland sessions don't support global hotkeys; switch to an X11 session |
| Other apps hear nothing | the output isn't a virtual cable, or the other app's mic isn't `CABLE Output`; check the hint in the bottom bar |
| Sudden sound while testing | see 8.1: output to the virtual cable, mute the mic, use a temp data directory |
| clippy reports lots of warnings | `-D warnings` turns warnings into errors; follow the suggestions, usually a line or two each |

---

## 11. Glossary

| Term | Meaning |
|---|---|
| PCM | uncompressed raw sample data (a list of numbers). An mp3 becomes PCM after decoding |
| Instance | when one sound plays several times at once, each playback is an instance |
| Varispeed | speed and pitch change together (like fast-forwarding a tape); how this project does speed |
| SPSC | Single Producer Single Consumer: one writer, one reader; a kind of ring buffer that needs no lock |
| WASAPI / ALSA / PulseAudio / PipeWire | the system audio interfaces of Windows / Linux |
| WebView2 / WebKitGTK | the system components that "show a web page inside a program" on Windows / Linux |
| Tauri | a framework for desktop app UIs using the system WebView, much smaller than Electron |
| axum / tokio | Rust's web server framework / async runtime |
| cpal | a cross-platform Rust audio library |
| Vite | a front-end bundler |
| CI | continuous integration: build, check and test automatically on every push |
| ADR | architecture decision record: "why it's designed this way" (see `docs/adr/`) |
| Feedback loop | the output is recorded back into the input, getting louder until it howls |
| Loopback device | devices like "Stereo Mix" that record everything the computer is playing |

---

**Next**: once you've done the trace in section 5, open the [Code walkthrough](code-walkthrough.md), which lists what
every struct and function is for and works as a dictionary; for the background of a particular design, see
[Soundboard architecture](soundboard-architecture.md) and the [ADRs](adr/).
