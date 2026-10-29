# FerryMic

**English** | [简体中文](README.md)

A small desktop "mic + soundboard" app: it mixes your microphone with sound effects and sends
the result to a virtual microphone. Voice chat, meeting and streaming apps use that virtual
microphone, so the people on the other end hear both your voice and the effects.

## Features

- ✅ **Desktop app**: its own window (Tauri + the system WebView), about 10 MB on Windows, no browser needed
- ✅ **Mic + effects mixing**: picks your hardware mic automatically; volume, mute, level meter; switch devices while running, auto-reconnect after a disconnect
- ✅ **Switchable output line**: choose the virtual cable in Settings without restarting; the bottom bar tells you which mic to pick in other apps
- ✅ **Monitor**: effects also play on your headphones / speakers so you can hear them (without your own voice, with its own volume)
- ✅ **Library = a folder on disk**: any location, subfolders are categories (nested); files you put there in the file manager show up after a rescan
- ✅ **Organise sounds**: create / rename / delete folders, drag a card onto a folder to move it; deleted files go to the recycle bin
- ✅ **Card quick controls**: loop / once (double-arrow icon), exclusive / overlap (dot / two overlapping circles) and speed all toggle with a click; volume opens a vertical slider
- ✅ **Defaults for new sounds**: exclusive, play once, 0 dB, 1.0× by default; editable in Settings, and can be applied to all existing sounds in one click
- ✅ **Seek bar**: drag the bar at the bottom of a playing card to jump
- ✅ **Global hotkeys**: record a key combo; works while other programs are full-screen
- ✅ **Appearance**: dark / light / follow system, 7 accent presets + custom; the app icon can follow the accent colour
- ✅ **Sound sites**: the globe button on the left lists free / open sound websites (Freesound, Pixabay, Mixkit, BBC and more), and you can add your own;
  in the app they open in a preview window, and audio files downloaded there go straight into the "Downloads" folder of your library
- ✅ **Languages**: Simplified Chinese / English, switch in Settings (follows the system by default); UI, error messages and tray menu switch together
- ✅ **Tray**: closing the window asks "minimize to tray" or "quit" (choice can be remembered)
- ✅ **Leak / feedback protection**: virtual cables that would loop back and "Stereo Mix"-style loopback devices are refused automatically
- ✅ **Linux**: creates a virtual microphone automatically (PipeWire / PulseAudio), no third-party software
- ✅ **Built-in help**: basics plus per-platform steps for connecting other apps
- ✅ **Phone remote**: one-click switch on the left (signal waves at the top right light up when on), scan the QR code to open it; protected by an access code, on/off without restarting
- ✅ **Browser version**: `soundbridge-server` serves the same UI to a browser

## Download

Get the file for your platform from [Releases](https://github.com/Time-Vent/FerryMic/releases):

| Platform | File | Notes |
|---|---|---|
| Windows | `FerryMic_x.y.z_x64-setup.exe` | Installer: installs WebView2 if needed, creates shortcuts, uninstall from "Settings → Apps" |
| Windows | `FerryMic_x.y.z_windows_x64.zip` | Portable: unzip and double-click `FerryMic.exe` |
| Ubuntu / Debian / Mint | `FerryMic_x.y.z_amd64.deb` | `sudo apt install ./FerryMic_x.y.z_amd64.deb`; dependencies are installed automatically |
| Other Linux | `FerryMic_x.y.z_amd64.AppImage` | `chmod +x` and run |

- Windows needs a virtual audio cable installed separately (e.g. [VB-Cable](https://vb-audio.com/Cable/)); on Linux the virtual microphone is created automatically.
- The program isn't code-signed, so on first run Windows SmartScreen may say "Windows protected your PC"; click "More info → Run anyway".
- Global hotkeys on Linux need an X11 session (on Wayland, trigger sounds from the window or the phone remote).
- Settings and the default library live in your user data directory (see "Data directory" below), so upgrading or moving the program keeps them.

### Releasing a new version (maintainers)

1. Update the version in `Cargo.toml` (`[workspace.package] version`) and `apps/app/tauri.conf.json`.
2. Commit, then tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. GitHub Actions (`.github/workflows/release.yml`) builds the files above on Windows and Ubuntu 22.04
   and uploads them to a **draft** release; check it, then click "Publish" on GitHub.

Every push to `main` and every PR runs `.github/workflows/ci.yml`: formatting, clippy and tests on Windows and Linux.

## Building from source

### Windows desktop app (recommended)

```powershell
powershell -ExecutionPolicy Bypass -File .\build-app.ps1
```

The output is in `dist-app\FerryMic\` (`FerryMic.exe` + `WebView2Loader.dll`, keep them together).
Double-click `FerryMic.exe` to run; the whole folder can be copied anywhere.

### Linux desktop app

```bash
# Debian / Ubuntu dependencies (install the equivalents on other distros)
sudo apt install build-essential pkg-config libasound2-dev libwebkit2gtk-4.1-dev \
     libgtk-3-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pulseaudio-utils
./build-app.sh            # output: dist-app/FerryMic/FerryMic
```

> The Linux version compiles and passes the tests in CI (Ubuntu 22.04), but hasn't been tried on a real Linux desktop yet (the development machine runs Windows).

### Browser version

```bash
cd apps/web && pnpm install && pnpm build && cd ../..
cargo run --release --bin soundbridge-server
# open http://127.0.0.1:8321 in a browser
```

`start-cable.ps1` / `start-speaker.ps1` are quick launchers for the browser version (output to the virtual cable / speakers).

## Using it

1. **Import sounds**: drag audio files into the window (or onto a folder on the left), or click "Import" at the top right.
   Files are saved into the library folder.
2. **Organise**: "New folder" on the left; the "⋯" next to a folder creates subfolders, renames or deletes; drag a card onto a folder to move it.
3. **Play**: click a card; while it plays, drag the seek bar to jump, hover the round button to stop; "Stop all" is at the top right.
4. **Adjust**: at the bottom of each card —
   - double-arrow icon: loop / once (click a looping card again to stop it);
   - dot / two overlapping circles: exclusive / overlap (exclusive stops other playing sounds);
   - volume: opens a vertical slider; speed: click for the next preset, right-click for the previous one;
   - "⋯" opens the full editor (name, folder, hotkey, delete).
5. **Microphone**: the mic bar at the bottom mutes, sets the volume and shows the level; a hardware mic is picked automatically, or choose one under Settings → "Audio chain".
6. **Connect other apps**: Settings → "Audio chain" → "Output line": pick a virtual cable (e.g. `CABLE Input`).
   The bottom bar then shows "Mic to pick in other apps: CABLE Output" — choose it as the microphone in the app you use.
   When the output is a speaker, the bottom bar turns into a yellow warning (other apps can't hear it).
7. **Monitor**: the headphones button at the bottom toggles the monitor, which also plays effects on your headphones; device and volume are in Settings.
8. **Phone remote**: click the phone icon on the left, turn it on and scan the QR code with your phone.
9. **Find sounds**: click the globe icon on the left for "Sound sites"; "Preview" opens a site in an app window where you can listen.
   Clicking the site's download button there saves audio files straight into the library's "Downloads" folder (other files go to the system downloads folder as usual);
   a notice at the top right shows the progress, and files already there aren't downloaded again. The globe button in the preview window's title bar (left of minimize) opens the page in your browser.
   With many sites, use the search box at the top right to filter by name, address or description.
   The licence notes in the list are summaries; each site's own terms apply.

**About microphone sources**:

- **Phone as a mic** (apps such as WO Mic): its PC driver adds a recording device; select it in Settings.
- **Bluetooth headsets**: Windows switches the headset to hands-free mode, where the mic is 8/16 kHz mono;
  FerryMic converts the sample rate automatically. Playback on the headset also drops to call quality — a limit of the Bluetooth protocol itself.
- If the device disconnects, "Waiting for the microphone…" is shown and it recovers automatically when reconnected, no restart needed.

**Input devices that are refused** (any vendor, judged by device name):

- The other end of the same virtual device as the output (e.g. `CABLE Output` → `CABLE Input`): it would loop back;
- Loopback devices such as Stereo Mix: they would record everything the computer plays.

A Bluetooth headset's mic and earpiece share a name but aren't virtual devices, so they aren't misjudged.

## Platforms

| Platform | Virtual microphone | Mic to pick in other apps |
|---|---|---|
| Windows | Install a virtual cable such as VB-Cable; under "Output line" pick `CABLE Input` | `CABLE Output` |
| Linux | **Created automatically** when FerryMic starts (needs `pactl`; PipeWire or PulseAudio), removed on exit | `FerryMic-Microphone` |
| macOS | Install BlackHole; under "Output line" pick `BlackHole 2ch` | `BlackHole 2ch` |

- Windows: keep the default playback device in "Sound settings" on your headphones or speakers, not `CABLE Input`, or everything the computer plays ends up in the mic.
- Linux: global hotkeys need an X11 session; on Wayland switch to X11, or trigger from the window / phone remote.
  cpal uses ALSA on Linux, so leave the mic on "Auto" and pick the default input device in the system sound settings.
- macOS: not tested and not packaged; on first use, allow microphone access under "Privacy & Security".

## Configuration

### Sound library

The library is a folder (default `<data directory>/sounds`; change it in Settings, optionally "moving the existing files there too"):

- Subfolders are categories and can be nested; files and folders starting with `.` are ignored.
- Supports WAV / MP3 / FLAC / OGG / M4A / AAC.
- Each sound's settings (loop / exclusive, volume, speed, hotkey) are stored in `.soundbridge.json` at the root,
  keyed by relative path, so the whole folder can be moved or synced to another computer.
- After adding, deleting or changing files in the file manager, click "Rescan" on the left (it also rescans when the window regains focus).
  Renaming or moving a file in the file manager loses that sound's settings (doing it inside the app doesn't).
- Large folders aren't all loaded into memory at once: they're preloaded in the background, and once about 768 MB is decoded the rest load on first play.

### Data directory

| Platform | Location |
|---|---|
| Windows | `%APPDATA%\FerryMic\` |
| Linux | `$XDG_DATA_HOME/FerryMic/` (default `~/.local/share/FerryMic/`) |
| macOS | `~/Library/Application Support/FerryMic/` |

`settings.json` stores the mic and output devices, monitor, phone remote (switch and access code), appearance, language, close behaviour,
library location, defaults for new sounds and your own sound sites. An old `library.json` is migrated into the library folder on first start
(the original is renamed to `library.v1.json.bak`).

**About the rename**: the project used to be called SoundBridge. On first start, FerryMic renames the old data directory (e.g. `%APPDATA%\SoundBridge`)
to `%APPDATA%\FerryMic`, keeping settings and library as they are (a library location inside the old directory is updated too). Internal crate names,
the `SOUNDBRIDGE_*` environment variables and the library's `.soundbridge.json` are unchanged.

### Environment variables (app and browser version)

| Variable | Effect |
|---|---|
| `SOUNDBRIDGE_OUTPUT_DEVICE` | Output device (substring match), e.g. `CABLE Input`; overrides the setting; with neither, the system default (on Linux, the auto-created virtual mic) |
| `SOUNDBRIDGE_INPUT_DEVICE` | Microphone (substring match); overrides the last choice |
| `SOUNDBRIDGE_MIC_MUTED` | `1` = start with the mic muted (avoids howling when outputting to speakers) |
| `SOUNDBRIDGE_LAN` | `1` / `0` = force the phone remote on / off (overrides the setting) |
| `SOUNDBRIDGE_PORT` | Port, default `8321` (the app picks another if it's taken); the phone remote uses "port + 1" |
| `SOUNDBRIDGE_DATA_DIR` | Data directory |

Microphone priority: `SOUNDBRIDGE_INPUT_DEVICE` > last choice > auto (the system default if it's a hardware mic, else the first hardware mic).
Output priority: `SOUNDBRIDGE_OUTPUT_DEVICE` > last choice > system default.

### Phone remote

**Off** by default. Click the phone icon on the left to turn it on — it **takes effect immediately** and the panel shows a QR code and the address
(phone and computer must be on the same Wi-Fi; the system may ask for firewall permission the first time).

- The phone remote listens on the local network on its own port (main port + 1), which closes when you turn it off; the desktop UI always listens on `127.0.0.1` only.
- The address carries an access code `?t=…`, remembered in a cookie on the phone after scanning; devices without the code can't open it.
  After "New access code", phones that scanned before have to scan again.
- Phones can't change computer-side options such as devices, settings or the library location; they can only play / stop / edit sounds.

## Architecture

```
apps/app (desktop app, Tauri)             apps/server (browser version entry)
   ├─ tray / close prompt / icon tint       │
   └─ native window ──loads──┐              │
                             ▼              ▼
              soundbridge_server (library): HTTP API + embedded front-end apps/web
                ├─ sound library (library.rs): folder on disk + .soundbridge.json
                ├─ loader thread (loader.rs): preload + on-demand load, the single trigger entry
                ├─ soundboard (soundboard crate): trigger semantics + mixing + seeking
                ├─ output thread (audio.rs): mixing callback + switching the output at runtime
                ├─ mic thread (mic.rs): capture + auto hardware mic + reconnect + input checks
                ├─ monitor thread (monitor.rs): effects-only mix → headphones / speakers (resampled)
                ├─ phone remote (remote.rs): LAN listener (port + 1) + access code + QR code
                ├─ global hotkey thread (hotkeys.rs): Win32 message loop
                ├─ Linux virtual mic (virtual_mic.rs): pactl null sink + remap source
                └─ audio output (audio-cpal): sound card / virtual cable
```

See [Soundboard architecture](docs/en/soundboard-architecture.md).

## REST API

| Method | Path | Description |
|---|---|---|
| GET | `/api/health` | Health check |
| GET | `/api/app` | Version, platform, data directory, library location, output device, appearance, phone remote, window options, virtual mic status |
| POST | `/api/settings` | Update settings `{theme_mode?, accent?, auto_virtual_mic?, tint_app_icon?, close_action?, language?, sound_defaults?, custom_sites?}` (`language`: `system` / `zh-CN` / `en`; `sound_defaults`: `{looping, exclusive, gain_db, speed}`; `custom_sites`: `[{name, url}]`) |
| POST | `/api/window` | Answer the close prompt `{action: "tray" \| "quit" \| "cancel", remember}` (desktop app) |
| POST | `/api/open-url` | Open an address `{url, title?, external}`: the app's preview window by default, the system browser with `external`; http(s) only |
| POST | `/api/pick-folder` | Show a "choose folder" dialog on the computer; returns `{path}` |
| GET | `/api/live` | Live state: `{playing: [{id, progress}], mic_level, mic_connected, close_prompt}` |
| GET | `/api/devices` | Device lists: outputs with `kind` (virtual / speaker) and `game_mic`, inputs with `virtual` |
| POST | `/api/output` | Switch the output line at runtime `{device: "name" \| null}`; the previous device is restored on failure |
| GET / POST | `/api/monitor` | Monitor status / settings `{enabled?, device?, gain_db?}` |
| GET / POST | `/api/remote` | Phone remote status / switch `{enabled?, regenerate?}`, immediate |
| GET | `/api/remote/qr.svg` | QR code of the phone remote address |
| GET / POST | `/api/mic` | Mic status / settings `{gain_db?, muted?}` |
| POST | `/api/mic/device` | Switch the mic `{device: "name" \| null}`; `null` = auto (prefer a hardware mic) |
| GET | `/api/library` | Library: `{root, folders, sounds}` |
| POST | `/api/library/rescan` | Rescan the disk |
| POST | `/api/library/location` | Change the library location `{path, move_contents}` |
| POST | `/api/library/open` | Open the library in the system file manager |
| POST / PATCH / DELETE | `/api/folders` | Create `{path}` / rename or move `{from, to}` / delete an empty folder `{path}` |
| GET | `/api/sounds` | List sounds |
| POST | `/api/sounds/import` | Upload (multipart: `file`, optional `folder`) |
| PATCH | `/api/sounds/:id` | Edit `{name?, folder?, looping?, exclusive?, gain_db?, speed?, hotkey?}`; `name` renames the file, `folder` moves it, `hotkey: null` clears |
| DELETE | `/api/sounds/:id` | Delete (the file goes to the recycle bin) |
| POST | `/api/sounds/:id/trigger` · `/stop` | Trigger / stop |
| POST | `/api/sounds/:id/seek` | Seek `{position: 0..1}` |
| POST | `/api/stop-all` | Stop everything |

## Development requirements

- **Rust stable**; on Windows **MinGW-w64** (`winget install BrechtSanders.WinLibs.POSIX.UCRT`)
- **Node.js LTS + pnpm** (front-end build; `winget install OpenJS.NodeJS.LTS`, `npm install -g pnpm`)
- Windows 10/11 + the WebView2 runtime (built into Win11); Linux needs WebKitGTK 4.1, the ALSA development package and `pactl`
- A virtual audio cable (Windows / macOS) so other apps can use the output as a microphone

> If the source path contains non-ASCII characters (e.g. a Chinese folder name), the GNU toolchain's `dlltool` may fail.
> Point `CARGO_TARGET_DIR` at an ASCII-only path.

**Known issues**:

- Building the Windows app with the GNU toolchain prints `.rsrc merge failure: multiple non-default manifests`
  (Tauri and the Rust standard library each bring a manifest that GNU ld can't merge); no functional impact found so far.
- The close prompt, tray and icon tint exist only in the desktop app; the browser version doesn't have these options.

## Directory layout

```
FerryMic/
├── crates/
│   ├── audio-types/     # plain data types
│   ├── platform-api/    # capability trait contracts
│   ├── dsp/             # fixed-size processors
│   ├── audio-graph/     # sequential processing chain
│   ├── audio-source/    # decoding + varispeed source (shared PCM, seeking)
│   ├── audio-engine/    # three executors + DriftController
│   ├── soundboard/      # soundboard core (many sounds + trigger semantics + seeking)
│   └── application/     # assembly layer for the CLI demo
├── adapters/
│   └── audio-cpal/      # AudioBackend implemented with cpal
├── apps/
│   ├── app/             # desktop app (Tauri shell)
│   ├── server/          # service library + browser entry (audio + hotkeys + library + API)
│   ├── web/             # UI (Vite + TS, shared by app / browser / phone)
│   └── desktop/         # CLI demo shell
├── build-app.ps1        # one-step Windows build
├── build-app.sh         # one-step Linux build
├── docs/                # architecture + code walkthrough + ADRs (Chinese; English in docs/en/)
└── tests/fixtures/audio/
```

## CLI tool (auxiliary)

```bash
soundbridge --list                                    # list devices
soundbridge in.wav --render out.wav --speed 2.0        # offline render
soundbridge in.wav --speed 1.5 --file-gain 0           # play a file + mic in real time
```

## Documentation

- [Soundboard architecture](docs/en/soundboard-architecture.md) (the main product)
- [Engine architecture](docs/en/architecture.md)
- [Code walkthrough](docs/en/code-walkthrough.md) (module by module, for learning)
- [ADRs](docs/en/adr/)

## License

MIT OR Apache-2.0
