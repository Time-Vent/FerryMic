# FerryMic 学习指南：从「会基本语法」到「能读懂、能测试、能改」

[English](en/learning-guide.md) | **简体中文**

> 写给只学过编程基本语法（变量、函数、循环、类 / 结构体）的计算机专业学生。
> 读完这份指南，你应该能：说清楚整个项目在干什么、知道每个文件夹和关键文件是什么、
> 找到读代码的起点、自己运行和测试、并动手做一些小改进。
>
> 配套文档：[代码走读](code-walkthrough.md)（逐模块的「字典」）、[音效板架构](soundboard-architecture.md)（设计细节）。
> 这份指南是「地图 + 路线」，那两份是「百科」。建议先读完本文，再按需查那两份。

## 目录

0. [怎么用这份指南](#0-怎么用这份指南)
1. [先用起来：这个项目到底做了什么](#1-先用起来这个项目到底做了什么)
2. [必备背景知识（只讲用得到的）](#2-必备背景知识只讲用得到的)
3. [工具链：每个命令在干什么](#3-工具链每个命令在干什么)
4. [文件夹地图：每个文件是什么](#4-文件夹地图每个文件是什么)
5. [跟着一次点击走完全程](#5-跟着一次点击走完全程)
6. [为什么这样设计：五个核心思想](#6-为什么这样设计五个核心思想)
7. [推荐阅读顺序：从哪里下手](#7-推荐阅读顺序从哪里下手)
8. [运行、调试与测试](#8-运行调试与测试)
9. [练习：由易到难动手改](#9-练习由易到难动手改)
10. [常见坑](#10-常见坑)
11. [术语表](#11-术语表)

---

## 0. 怎么用这份指南

建议按下面四个阶段来，每个阶段都有明确的「完成标志」：

| 阶段 | 读哪几节 | 大概时间 | 完成标志 |
|---|---|---|---|
| ① 看懂全局 | 1、2、4 | 半天 | 能画出「声音从麦克风到其他软件」的流程图，能说出每个顶层文件夹是干什么的 |
| ② 跑起来 | 3、8 | 半天 | 能从源码编译出 App、运行测试、用浏览器版调试 |
| ③ 读懂主线 | 5、6、7 | 2～3 天 | 能顺着一次点击，从界面代码一路找到发出声音的那一行 |
| ④ 动手改 | 9、10 | 持续 | 完成 2～3 个练习，测试全部通过 |

**一个心态**：这个项目有两万多行代码，你**不需要全部看懂**。专业程序员面对陌生项目也是「先抓主线，用到哪里查到哪里」。

---

## 1. 先用起来：这个项目到底做了什么

### 1.1 一句话

FerryMic 是一个**桌面「麦克风 + 音效板」**：把你的麦克风和音效混在一起，送进一个「虚拟麦克风」，别的软件（语音、会议、直播）把它当麦克风用，对方就能同时听到你说话和音效。

### 1.2 声音的旅程（整个项目最重要的一张图）

```
 你的真实麦克风 ──▶ ┌──────────────── FerryMic ────────────────┐
                    │                                          │
 音效文件 (mp3/wav) ─▶  解码成数字 ─▶ 混音（相加）─▶ 限幅 ───────┼──▶ 虚拟音频线 CABLE Input
                    │                    │                     │         │
                    │                    └─▶ 只有音效的一份 ──────┼──▶ 你的耳机（试听）
                    └──────────────────────────────────────────┘         │
                                                                           ▼
                                                     其他软件把 CABLE Output 当麦克风
```

**虚拟音频线**（如 VB-Cable）是一个假的声卡：往它的「输入端」（CABLE Input）放声音，就能从它的「输出端」（CABLE Output）当麦克风录到。Windows 自己做不到这一点，所以要装第三方的；Linux 版 FerryMic 会自己用系统命令造一个。

### 1.3 程序长什么样（三层）

```
┌─────────────────────────────────────────────┐
│ 界面（网页技术：HTML + CSS + TypeScript）     │  apps/web
│   看到的一切：卡片、按钮、设置……              │
└───────────────▲─────────────────────────────┘
                │ HTTP 请求（和访问网站一样）
┌───────────────┴─────────────────────────────┐
│ 后端服务（Rust）                              │  apps/server
│   音效库、加载、快捷键、麦克风、输出、手机遥控  │
└───────────────▲─────────────────────────────┘
                │ 函数调用
┌───────────────┴─────────────────────────────┐
│ 音频核心库（Rust）                            │  crates/*、adapters/*
│   解码、变速、混音、和声卡打交道               │
└─────────────────────────────────────────────┘
外面再包一层「桌面 App 外壳」（Tauri）：           apps/app
   提供窗口、托盘图标，窗口里显示的就是上面那个网页
```

**为什么界面用网页？** 网页做界面最方便、最好看，而且同一套界面还能直接给手机用（手机遥控就是用手机浏览器打开同一个网页）。

---

## 2. 必备背景知识（只讲用得到的）

### 2.1 数字音频的 10 个概念

| 概念 | 一句话解释 | 在代码里长什么样 |
|---|---|---|
| **采样（sample）** | 声音是空气振动，电脑每隔一小段时间记录一次振动的位置，记下来的一个数就是一个采样 | 一个 `f32`，范围 -1.0 ~ 1.0 |
| **采样率** | 每秒记录多少次。48000 Hz = 每秒 48000 个采样 | `sample_rate: u32`、`SampleRate(48000)` |
| **声道** | 单声道 1 路，立体声 2 路（左、右） | `channels` |
| **帧（frame）** | 同一时刻所有声道的采样合在一起叫一帧。立体声一帧 = 2 个采样 | `frames`；`samples = frames × channels` |
| **交错（interleaved）** | 立体声在数组里排成 `左 右 左 右 ……` | 所有 `&[f32]` 缓冲都是交错的 |
| **混音** | 把几路声音**逐个采样相加** | `out[i] += sound[i] * gain` |
| **增益 / dB** | 音量。dB 是对数单位：+6 dB ≈ 音量翻倍，0 dB = 不变，-60 dB ≈ 静音 | `gain_db`，`db_to_linear()` 换算 |
| **限幅（clip）** | 相加后可能超过 1.0，超出部分会爆音，所以要截到 [-1, 1] | `s.clamp(-1.0, 1.0)` |
| **重采样** | 文件是 44100 Hz，声卡是 48000 Hz，要「换算」成同一个采样率 | `rate_ratio`、线性插值 |
| **缓冲区 + 回调** | 声卡不会一个采样一个采样地要，而是每隔几毫秒说「给我 480 帧」，调用你写的一个函数（回调）来填 | `let callback = move \|data: &mut [f32]\| { ... }` |

**最关键的一点**：声卡调用回调是有**截止时间**的。48000 Hz、每次 480 帧，你只有 **10 毫秒**把数据填好；晚了就会「卡顿 / 爆音」。这条规则决定了整个项目的结构（见第 6 节）。

### 2.2 并发：为什么有这么多线程

一个程序里可以同时有多个「执行线路」（线程）。FerryMic 有十个左右：

- **声卡回调线程**：只负责填声音，必须快。
- **加载线程**：读文件、解码（慢，可能要几百毫秒）。
- **麦克风线程**、**快捷键线程**、**网页服务线程**……

线程之间传数据有三种工具，你会反复看到：

| 工具 | 用途 | 类比 |
|---|---|---|
| **通道（channel）** | 一个线程「发消息」，另一个「收消息」 | 传纸条 |
| **原子量（Atomic）** | 一个数字，多个线程可以安全地同时读写，不用加锁 | 公告板上的一个数字 |
| **环形缓冲（ring buffer，SPSC）** | 一个线程不停写声音、另一个不停读，首尾相接循环使用 | 回转寿司传送带 |
| 锁（Mutex） | 同一时间只允许一个线程碰某个数据 | 洗手间的门锁 —— **回调线程里禁止使用**，因为等锁可能超过 10 ms |

### 2.3 Rust 必要概念速览

你只要先认识这些，看代码时能对上号就行：

```rust
// Result：可能失败的函数返回 Ok(值) 或 Err(错误)；末尾的 ? 表示「出错就直接返回错误」
fn load(path: &str) -> Result<Sound, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // ...
}

// Option：可能没有值，Some(值) 或 None
let device: Option<String> = None; // None 表示「用系统默认」

// struct + impl：数据 + 方法（相当于其他语言的类）
struct Player { /* ... */ }
impl Player {
    pub fn trigger(&self, id: SoundId) { /* ... */ }
}

// trait：接口（规定「必须有哪些方法」）
pub trait AudioBackend { fn open_output(/* ... */); }

// Arc<T>：多个线程共享同一个东西（引用计数）；Mutex<T>：加锁访问
let shared = Arc::new(Mutex::new(Vec::new()));

// 闭包：匿名函数，move 表示把用到的变量「搬」进去
let callback = move |data: &mut [f32]| { /* 填声音 */ };

// async / await：异步，等网络时不占着线程。网页服务（axum + tokio）用它
async fn trigger_sound(/* ... */) -> StatusCode { /* ... */ }
```

- **所有权与借用**：Rust 在编译时保证「同一份数据不会被两个线程同时乱改」。编译不过时，错误信息通常会直接告诉你怎么改，**认真读编译器的报错**是学 Rust 最快的方法。
- **crate**：Rust 的「包」，一个 crate 就是一个库或程序。**workspace**：把多个 crate 放在一个仓库里统一管理。
- **宏**：名字后面带 `!` 的，比如 `println!`、`format!`，本项目自己定义了 `tr!` / `trf!`（中英文提示）。

### 2.4 前端必要概念速览

- **HTML** 定义有哪些元素（按钮、卡片），**CSS** 定义长什么样，**TypeScript** 是带类型的 JavaScript，定义点了会发生什么。
- `fetch('/api/library')`：向后端发 HTTP 请求，拿回 **JSON**（一种文本格式的数据，`{"id": 1, "name": "鼓掌"}`）。
- **轮询**：每隔 100 毫秒问一次后端「现在在放什么？」，界面据此更新进度条。
- 本项目**没用 React / Vue 这类框架**，直接操作页面元素（DOM），所以代码更直白，适合学习。

### 2.5 HTTP API 是什么

后端开放了一组「网址 + 动作」，比如：

```
POST /api/sounds/3/trigger   → 播放 3 号音效
GET  /api/library            → 返回音效列表（JSON）
```

界面、手机、甚至你自己用 `curl` 命令，都通过这组网址控制 FerryMic。**完整列表见 README 的「REST API」一节**。

---

## 3. 工具链：每个命令在干什么

### 3.1 需要安装的东西

| 工具 | 作用 | 安装 |
|---|---|---|
| **Rust**（rustup、cargo） | 编译 Rust 代码；cargo 是 Rust 的「构建 + 包管理 + 测试」工具 | rustup.rs |
| **MinGW-w64**（Windows） | 本项目在 Windows 上用的 GNU 编译工具链 | `winget install BrechtSanders.WinLibs.POSIX.UCRT` |
| **Node.js + pnpm** | 构建网页界面；pnpm 是 npm 的替代品，装依赖更快 | `winget install OpenJS.NodeJS.LTS`，`npm install -g pnpm` |
| **Git** | 版本管理 | git-scm.com |
| VB-Cable（可选） | 虚拟音频线，测试「其他软件能不能收到」时需要 | vb-audio.com |

### 3.2 常用命令

在项目根目录（`FerryMic/`）运行：

| 命令 | 做什么 | 什么时候用 |
|---|---|---|
| `cargo build` | 编译整个 workspace（调试版，快但未优化） | 改完 Rust 代码看能不能编译 |
| `cargo test` | 运行所有单元测试 | 改完代码必跑 |
| `cargo test -p soundbridge-server` | 只测某一个 crate | 只改了 server 时，更快 |
| `cargo test exclusive` | 只跑名字里带 `exclusive` 的测试 | 调试单个测试 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 代码检查（比编译器更挑剔，警告也算失败） | 提交前 |
| `cargo fmt --all` | 自动格式化代码 | 提交前 |
| `cargo run --bin soundbridge-server` | 运行浏览器版服务 | 调试界面和 API（见 8.2） |
| `cargo clean` | 删除编译缓存 `target/` | 磁盘空间不够时 |

在 `apps/web/` 里运行：

| 命令 | 做什么 |
|---|---|
| `pnpm install` | 安装前端依赖（到 `node_modules/`） |
| `pnpm build` | 把 TypeScript 编译、打包到 `dist/`（后端会把这个文件夹嵌进程序） |
| `pnpm exec tsc --noEmit` | 只做 TypeScript 类型检查 |

打包整个桌面 App：`powershell -ExecutionPolicy Bypass -File .\build-app.ps1`（Windows）或 `./build-app.sh`（Linux）。

### 3.3 GitHub 上自动做的事

- **CI**（`.github/workflows/ci.yml`）：每次推送，在 Windows 和 Linux 云端机器上跑格式检查、clippy、测试。
- **Release**（`.github/workflows/release.yml`）：推送 `v0.1.0` 这样的标签时，自动打出安装包，放进草稿 Release。

---

## 4. 文件夹地图：每个文件是什么

### 4.1 顶层

```
FerryMic/
├── Cargo.toml          ← workspace 配置：有哪些 crate、统一的依赖版本、发布版优化选项
├── Cargo.lock          ← 【自动生成】锁定每个依赖的精确版本，保证大家编译结果一致。要提交，不要手改
├── rust-toolchain.toml ← 指定用 stable 版 Rust + Windows GNU 目标
├── rustfmt.toml / clippy.toml / deny.toml ← 格式化、代码检查、依赖许可检查的规则
├── justfile            ← 常用命令的快捷方式（需要装 just 工具，可选）
├── build-app.ps1 / build-app.sh ← 一键打包桌面 App
├── start-cable.ps1 / start-speaker.ps1 ← 快速启动浏览器版（输出到虚拟线 / 扬声器）
├── README.md / README.en.md ← 项目说明（中 / 英）
├── .gitignore          ← 告诉 git 哪些文件不要提交
├── .github/workflows/  ← CI 与自动发布配置
├── crates/             ← 音频核心库（纯 Rust，和界面无关）
├── adapters/           ← 和真实声卡打交道的实现
├── apps/               ← 能运行的程序
├── docs/               ← 文档
├── tests/fixtures/     ← 测试用的音频素材
│
├── target/             ← 【自动生成】Rust 编译输出与缓存，可能好几 GB。不提交，可以随时删（cargo clean）
└── dist-app/           ← 【自动生成】build-app 打包好的 exe。不提交
```

### 4.2 `crates/`：音频核心（按依赖从底到顶）

| crate | 一句话 | 值得看的文件 |
|---|---|---|
| `audio-types` | 最底层的数据类型（采样率、声道数、增益……），不依赖任何别的 crate | `src/lib.rs` |
| `platform-api` | 定义「声卡后端」必须提供哪些功能（trait），不含实现 | `src/lib.rs` |
| `dsp` | 音效处理器：增益、限幅、均衡 | `gain.rs`、`limiter.rs`、`biquad.rs` |
| `audio-graph` | 把多个处理器串成一条链 | `src/lib.rs` |
| `audio-source` | 读音频文件、解码、**变速**（倍速播放就靠它） | `varispeed.rs`（核心）、`audio_file.rs` |
| `audio-engine` | 早期引擎（给命令行演示用）：执行器划分、时钟漂移控制 | `executors.rs`、`clock.rs` |
| `application` | 把早期引擎装配起来（给命令行演示用） | `app.rs` |
| **`soundboard`** | **音效板核心：管理很多音效、播放 / 停止 / 循环 / 独占、混音**。主产品最重要的纯逻辑 | `src/lib.rs`（**推荐第一个读**） |

### 4.3 `adapters/audio-cpal/`：连接真实声卡

用开源库 **cpal** 实现 `platform-api` 规定的接口。Windows 上走 WASAPI，Linux 上走 ALSA。关键函数：`resolve_output`（按名字找设备）、`build_output`（开始往声卡送声音）、`build_input`（开始从麦克风收声音）。

### 4.4 `apps/`：能运行的程序

| 文件夹 | 是什么 |
|---|---|
| `apps/server/` | **后端服务库 + 浏览器版程序**。主产品的大脑，见下表 |
| `apps/web/` | **界面**（网页） |
| `apps/app/` | **桌面 App 外壳**（Tauri）：窗口、托盘、关闭询问、图标变色、音效资源预览窗口 |
| `apps/desktop/` | 早期的命令行演示程序（`soundbridge --list`） |
| `apps/mobile/` | 移动端占位，没有代码 |

**`apps/server/src/` 逐个文件**（主产品的核心，建议重点看）：

| 文件 | 负责什么 |
|---|---|
| `main.rs` | 浏览器版的入口，只有十几行：读环境变量 → 调用 `run()` |
| `lib.rs` | **启动流程 `run()`**：读设置 → 打开音效库 → 启动各个线程 → 开始提供网页服务 |
| `api.rs` | **所有 HTTP 接口**：每个 `/api/...` 网址对应一个 `async fn`。界面的每个按钮最终都落到这里 |
| `library.rs` | 音效库 = 磁盘文件夹：扫描、导入、改名、移动、删除，以及 `.soundbridge.json` 的读写 |
| `loader.rs` | 加载线程：后台把音效解码进内存；`Player::trigger` 是**播放的唯一入口** |
| `audio.rs` | **输出线程和声卡回调**：每 10 毫秒混一次音，送给声卡 |
| `mic.rs` | 麦克风线程：自动选麦克风、断线重连、防反馈检查 |
| `monitor.rs` | 试听：把只有音效的那份声音再送到耳机 |
| `hotkeys.rs` | 全局快捷键 |
| `passthrough.rs` | 「不拦截」组合键：用 Windows 底层钩子旁听按键，按键照样传给游戏 |
| `remote.rs` | 手机遥控：局域网监听、访问码、二维码 |
| `settings.rs` | 设置的保存与读取（`settings.json`） |
| `i18n.rs` | 中英文提示语 |
| `virtual_mic.rs` | Linux 自动创建虚拟麦克风 |

**`apps/web/` 逐个文件**：

| 文件 | 负责什么 |
|---|---|
| `index.html` | 页面骨架：所有对话框、按钮、设置项的 HTML |
| `src/main.ts` | **界面逻辑**（最大的文件）：渲染卡片、处理点击、轮询状态、设置页 |
| `src/api.ts` | 所有后端接口的 TypeScript 类型和调用函数（看它就知道前后端交换什么数据） |
| `src/i18n.ts` | 中→英翻译词典 |
| `src/icons.ts` | 所有图标（SVG） |
| `src/sites.ts` | 内置的音效资源网站列表 |
| `src/style.css` | 所有样式、深色 / 浅色主题 |
| `dist/` | 【自动生成，但提交了】`pnpm build` 的输出，会被嵌进后端程序 |
| `node_modules/` | 【自动生成】前端依赖，不提交 |
| `package.json` / `pnpm-lock.yaml` / `pnpm-workspace.yaml` | 前端依赖清单 / 锁定版本 / pnpm 设置 |
| `vite.config.ts` / `tsconfig.json` | 打包工具与 TypeScript 的配置 |

**`apps/app/`**：

| 文件 | 负责什么 |
|---|---|
| `src/main.rs` | 桌面外壳的全部逻辑：启动后端、建窗口、托盘、关闭询问、图标变色、预览窗口、下载入库 |
| `tauri.conf.json` | 程序名、图标、打包设置 |
| `loading/index.html` | 后端启动失败时显示的错误页 |
| `loading/titlebar.html` | 预览窗口自己画的标题栏 |
| `preview/preview.js` | 注入预览网页的「下载提示」脚本 |
| `capabilities/` | Tauri 的权限配置 |
| `icons/` | 程序图标 |
| `gen/schemas/` | 【自动生成】Tauri 的配置格式说明 |
| `build.rs` | 编译前运行的小脚本（Tauri 用它读配置、嵌图标） |

### 4.5 运行时产生的文件（不在项目里）

| 位置 | 内容 |
|---|---|
| `%APPDATA%\FerryMic\settings.json` | 所有设置 |
| `%APPDATA%\FerryMic\sounds\` | 默认音效库 |
| `音效库\.soundbridge.json` | 每个音效的设置（循环、音量、快捷键……） |

---

## 5. 跟着一次点击走完全程

这是读懂项目最有效的方法：**选一个用户动作，顺着代码一路追下去**。下面追「点一下卡片 → 听到声音」。

### 第 1 站：界面（TypeScript）

`apps/web/src/main.ts`，函数 `renderPad` 里：

```ts
const fire = () => {
  if (!s.error) api.trigger(s.id).catch((e) => toast(errorText(e), 'error'));
};
pad.addEventListener('click', fire);
```

点击卡片 → 调 `api.trigger(3)`。在 `apps/web/src/api.ts` 里能看到它就是：

```ts
trigger: (id: number) => request<void>(`/api/sounds/${id}/trigger`, { method: 'POST' }),
```

也就是向后端发一个 `POST /api/sounds/3/trigger`。

### 第 2 站：HTTP 接口（Rust，axum）

`apps/server/src/api.rs`，路由表里有一行：

```rust
.route("/sounds/:id/trigger", post(trigger_sound))
```

意思是「收到这个网址的 POST，就调用 `trigger_sound`」：

```rust
async fn trigger_sound(State(state): State<AppState>, Path(id): Path<SoundId>) -> StatusCode {
    state.player.trigger(id);
    StatusCode::OK
}
```

`Path(id)` 自动从网址里取出 `3`；`State(state)` 是所有接口共享的「全局状态」（音效库、播放器、设置……）。

### 第 3 站：播放器（加载线程）

`apps/server/src/loader.rs`：

```rust
pub fn trigger(&self, id: SoundId) {
    if self.is_loaded(id) {
        let _ = self.cmd_tx.send(BoardCommand::Trigger { sound_id: id });
    } else {
        let _ = self.urgent_tx.send(Job::LoadAndTrigger(id));
    }
}
```

- 已经解码在内存里了：**通过通道**给声卡线程发一条「播放 3 号」的命令。
- 还没解码：交给加载线程先解码（慢的活儿不能让声卡线程做），解码完再发命令。

### 第 4 站：声卡回调（每 10 毫秒一次）

`apps/server/src/audio.rs`，`let callback = move |data: &mut [f32]| { ... }`：

```rust
// 1. 取出最多 32 条命令执行（比如刚才的「播放 3 号」）
for _ in 0..32 {
    match cmd_rx.try_recv() { Ok(cmd) => board.apply_command(cmd), Err(_) => break }
}
// 2. 把所有正在播放的音效混到 data 里
board.fill(data, frames, out_ch);
playing.publish(&board);             // 告诉界面「哪些在放、放到哪了」
// 3. 只有音效的一份给试听
monitor_tap.tap(data, frames, out_ch);
// 4. 加上麦克风
mic_reader.mix_into(data, frames, out_ch, &mic);
// 5. 限幅
for s in data.iter_mut() { *s = s.clamp(-1.0, 1.0); }
```

`data` 填完后，cpal 把它交给声卡（虚拟音频线），其他软件就「听到」了。

### 第 5 站：混音核心

`crates/soundboard/src/lib.rs`：

- `apply_command` 收到 `Trigger` → 调 `trigger`：按播放方式处理（循环中再按就停、独占就先清掉别的），然后新建一个「播放实例」。
- `fill`：遍历所有实例，每个实例从自己的位置往后取声音（按倍速和采样率换算步进），乘音量后**加**到输出上。

### 第 6 站：界面怎么知道它在放

- 回调里的 `playing.publish(&board)` 把「3 号，播放到 42%」写进一组**原子量**（不加锁）。
- 界面每 100 毫秒请求一次 `GET /api/live`（`main.ts` 的 `startPolling`），拿到进度后更新进度条（`applyPlaying`）。

**到这里你已经走通了整条主线。** 用同样的方法再追两个动作，就基本掌握了整个项目：

- 「拖文件进窗口导入」：`main.ts` 的 `importFiles` → `POST /api/sounds/import` → `library.rs` 的 `import`。
- 「麦克风的声音怎么进来」：`mic.rs` 的 `open` → `MicCapture::push`（麦克风回调写环形缓冲）→ `audio.rs` 的 `MicReader::mix_into`（声卡回调从环里读）。

---

## 6. 为什么这样设计：五个核心思想

### 6.1 实时安全：回调里「四不」

声卡回调必须在截止时间前完成，所以回调里**不分配内存、不加锁、不读写文件、不 panic**。这直接导致了：

- 解码放在加载线程（读文件、解码都很慢）；
- 回调和别的线程之间只用**通道、原子量、环形缓冲**通信；
- 音效 PCM 用 `Arc` 共享，播放时只增加引用计数，不复制。

**读代码时的检验方法**：看到一段代码在回调里，就检查它有没有违反「四不」。

### 6.2 每种慢活儿一个线程

加载线程、麦克风线程、试听线程、快捷键线程……各管一摊，通过「请求-回复」通信（比如 `MicRequest::Select { device, reply }`：发请求时附上一个回信通道，对方处理完把结果寄回来）。这样任何一个慢操作都不会卡住别人。

### 6.3 前后端分离：界面只是一个网页

后端只提供 HTTP 接口，不关心界面长什么样。好处：桌面窗口、浏览器、手机用的是**同一套界面、同一套接口**；你也可以用 `curl` 或脚本直接控制它。

### 6.4 磁盘就是数据库

音效库直接就是一个文件夹，子文件夹就是分类，每个音效的设置存在 `.soundbridge.json`。没有用数据库，用户在资源管理器里就能管理文件，整个文件夹也能直接拷到别的电脑。

### 6.5 能测的逻辑和平台代码分开

`crates/soundboard` 是纯逻辑：给它一个假的输出缓冲，就能测「触发后有没有声音」，**不需要真的声卡**。所以项目里大部分测试都能在没有声卡的 CI 云端机器上运行。你写新功能时也应该尽量这样分：把「算」的部分和「跟硬件打交道」的部分分开。

---

## 7. 推荐阅读顺序：从哪里下手

| 顺序 | 读什么 | 为什么从这里读 | 读的时候重点看 |
|---|---|---|---|
| 1 | `crates/soundboard/src/lib.rs` | 纯逻辑、不依赖硬件、有测试，最容易读懂 | `PlayMode`、`apply_command`、`trigger`、`fill`；**先读文件末尾的测试**，测试就是「用法示例」 |
| 2 | `crates/audio-source/src/varispeed.rs` | 理解「倍速播放」和「线性插值」 | `fill`、`interp`、`progress` |
| 3 | `apps/web/src/api.ts` | 一个文件看清前后端交换的所有数据 | 各个 `interface` 和 `api` 对象 |
| 4 | `apps/server/src/api.rs` | 每个接口在后端怎么处理 | 路由表（`fn router`）→ 挑几个处理函数看 |
| 5 | `apps/server/src/lib.rs` 的 `run()` | 程序启动时按什么顺序做了什么 | 一步步对照 [代码走读 §12](code-walkthrough.md#12-server音效板服务库) |
| 6 | `apps/server/src/audio.rs` | 实时回调长什么样 | `callback` 闭包、`PlayingSet` |
| 7 | `apps/server/src/loader.rs`、`library.rs` | 加载与音效库 | `Player::trigger`、`Worker::run`、`scan` |
| 8 | `apps/web/src/main.ts` | 界面逻辑（很长，挑着看） | `renderPad`、`renderControls`、`startPolling` |
| 9 | 其余按兴趣 | `mic.rs`（设备处理）、`remote.rs`（安全）、`apps/app/src/main.rs`（桌面外壳） | |

**读代码的小技巧**：

- 在 VS Code 里装 **rust-analyzer** 插件，按住 Ctrl 点函数名就能跳到定义，鼠标悬停能看到类型。这是读 Rust 代码最重要的工具。
- 遇到看不懂的函数，先看它的**注释**（本项目每个函数都有中英文注释）和**调用它的地方**。
- 全局搜索（Ctrl+Shift+F）一个界面上的中文文字，就能找到对应的前端代码，再顺着 `api.xxx` 找到后端。

---

## 8. 运行、调试与测试

### 8.1 安全测试的原则：别让电脑突然出声

调试音频程序时很容易误触发音效。建议：

- 输出设到虚拟音频线（`SOUNDBRIDGE_OUTPUT_DEVICE=CABLE Input`），而不是扬声器；
- 启动时静音麦克风（`SOUNDBRIDGE_MIC_MUTED=1`），避免扬声器和麦克风之间形成啸叫；
- 用单独的数据目录（`SOUNDBRIDGE_DATA_DIR=某个临时文件夹`），不要弄乱自己平时用的设置和音效库。

### 8.2 用浏览器版调试（推荐）

浏览器版和桌面 App 是同一套代码，但调试方便得多：

```powershell
cd apps\web; pnpm install; pnpm build; cd ..\..
$env:SOUNDBRIDGE_DATA_DIR = "$env:TEMP\ferrymic-dev"
$env:SOUNDBRIDGE_OUTPUT_DEVICE = "CABLE Input"
$env:SOUNDBRIDGE_MIC_MUTED = "1"
$env:SOUNDBRIDGE_PORT = "8341"
cargo run --bin soundbridge-server
```

然后浏览器打开 `http://127.0.0.1:8341`，按 **F12** 打开开发者工具：

- **Console**：看前端报错；
- **Network**：看每个按钮发了什么请求、后端返回了什么 —— 这是理解前后端交互最直观的方式。

改了前端代码后：重新 `pnpm build`，刷新浏览器即可（调试版后端直接读 `dist/` 文件夹）。改了 Rust 代码后：停掉服务，重新 `cargo run`。

### 8.3 看日志

后端用 `log::info!` / `log::warn!` 输出日志，默认显示 info 级别。想看更详细的：

```powershell
$env:RUST_LOG = "debug"; cargo run --bin soundbridge-server
```

### 8.4 用 curl 直接调接口

不用界面也能测试后端：

```bash
curl http://127.0.0.1:8341/api/library
```

```bash
curl -X POST http://127.0.0.1:8341/api/sounds/1/trigger
```

### 8.5 运行和编写测试

```bash
cargo test --workspace
```

测试写在每个文件末尾的 `#[cfg(test)] mod tests { ... }` 里。看一个真实的例子（`crates/soundboard/src/lib.rs`）：

```rust
#[test]
fn trigger_real_sound_produces_audio() {
    let dir = temp_dir("soundboard-test");
    let wav = make_test_wav(&dir, "sine.wav", 48_000, 4800); // 现场生成一个正弦波文件

    let mut board = SoundBoard::new(4800, 48_000);
    add(&mut board, 1, &wav, TriggerMode::OneShot);

    board.apply_command(BoardCommand::Trigger { sound_id: 1 });
    assert_eq!(board.instance_count(), 1);                     // 断言：有 1 个在放

    let mut out = vec![0.0f32; 512];
    board.fill(&mut out, 512, 1);                             // 混一次音
    let peak = out.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
    assert!(peak > 0.01, "预期有声音，实际峰值 peak={peak}");   // 断言：确实有声音
}
```

测试的套路永远是三步：**准备（造数据）→ 执行（调用函数）→ 断言（检查结果）**。写新测试时，复制一个相似的测试改一改是最快的。

### 8.6 提交前的检查清单

```bash
cargo fmt --all
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

前端改了的话，再在 `apps/web` 里跑 `pnpm exec tsc --noEmit` 和 `pnpm build`。这几项全部通过，推到 GitHub 后 CI 才会是绿色。

---

## 9. 练习：由易到难动手改

每个练习都标了**涉及的文件**和**提示**。建议每做完一个都跑一遍 8.6 的检查。

### ★ 入门（只改一两个文件）

1. **改一句界面文字并加上英文翻译**
   - 文件：`apps/web/index.html`、`apps/web/src/i18n.ts`
   - 提示：中文原文就是词典的「键」；改了中文要同时改 `i18n.ts` 里的键。在设置里切换语言检查效果。

2. **增加一个倍速档位（比如 3.0×）**
   - 文件：`apps/web/src/main.ts` 的 `SPEED_PRESETS`
   - 提示：后端对倍速有范围限制，找找 `speed` 在 `api.rs` 和 `audio-types` 里被钳制到多少，必要时一起改，并想想为什么要限制。

3. **给 `SoundDefaults::clamped` 补一个测试**
   - 文件：`apps/server/src/library.rs`
   - 提示：仿照已有的 `defaults_are_clamped`，测负无穷、超大倍速等边界值。

### ★★ 进阶（前后端都要动）

4. **新增接口 `GET /api/stats`，返回音效总数和已加载数**
   - 文件：`apps/server/src/api.rs`（路由 + 处理函数）、`apps/web/src/api.ts`（类型 + 调用）
   - 提示：仿照 `get_live`；`state.library.entries()` 拿全部音效，`state.player.is_loaded(id)` 判断是否已加载。用 curl 测试。

5. **在设置页「关于」里显示这个统计**
   - 文件：`index.html`、`main.ts`
   - 提示：在 `openSettings` 里调用新接口，结果写进一个新元素；记得加翻译。

6. **快捷键冲突提示更友好**：被占用时，提示是哪个音效占用的（显示名字而不是编号）
   - 文件：`apps/server/src/hotkeys.rs`、`api.rs`
   - 提示：错误信息里现在是 `#{other}`；想办法在 API 层把 id 换成名字。

### ★★★ 挑战（涉及实时音频）

7. **停止时淡出**：点停止时不要「咔」一下断掉，而是 50 毫秒内音量渐变到 0
   - 文件：`crates/soundboard/src/lib.rs`
   - 提示：给 `Instance` 加一个「淡出剩余帧数」字段；`Stop` 命令不再直接删除实例，而是开始淡出；在 `fill` 里按剩余帧数逐帧乘一个递减系数，减到 0 再删除。**不能在回调里分配内存**。先写测试：停止后的前几个采样不为 0、50 毫秒后全为 0。

8. **全局主音量**：一个滑块同时控制所有音效的总音量（不影响麦克风）
   - 文件：`audio.rs`（回调里在 `board.fill` 之后乘一个系数）、`api.rs`、`settings.rs`、前端
   - 提示：系数用 `AtomicU32` 存 f32 的位模式（参考 `MicControl` 的 `gain_db`），回调里只读、不加锁。

9. **音量电平表**：在底部条显示音效混音的实时音量（像麦克风电平表那样）
   - 提示：参考 `MicControl::update_level` 的「快升慢降」，在回调里算 `board.fill` 之后的峰值，经 `/api/live` 传给前端。

---

## 10. 常见坑

| 现象 | 原因与解决 |
|---|---|
| 打包时报「文件被占用」 | 有个 FerryMic 还在运行（可能最小化在托盘），退出后再打包 |
| 双击新版 exe 却打开了旧窗口 | 单实例机制：已有一个在运行时，再启动只会唤醒旧的。先完全退出 |
| `cargo build` 很慢、`target/` 好几 GB | 正常，第一次要编译几百个依赖。之后只编译改动部分 |
| 改了前端，桌面 App 里没变化 | 发布版把前端**编译进了 exe**，要重新 `pnpm build` 再重新打包；调试时用浏览器版更方便 |
| PowerShell 改文件后中文变乱码 | Windows PowerShell 5.1 默认按 GBK 读写文件。改源码请用编辑器，不要用 `Get-Content` / `Set-Content` |
| 在 Linux 上快捷键没反应 | Wayland 会话不支持全局快捷键，换成 X11 会话 |
| 其他软件里收不到声音 | 输出没选虚拟音频线，或其他软件的麦克风没选 `CABLE Output`；看底部条的提示 |
| 测试时突然出声 | 见 8.1：输出到虚拟线、麦克风静音、用临时数据目录 |
| clippy 报一堆警告 | `-D warnings` 把警告当错误；照着提示改，通常一两行就能修好 |

---

## 11. 术语表

| 术语 | 解释 |
|---|---|
| PCM | 未压缩的原始采样数据（一串数字）。mp3 解码后就是 PCM |
| 实例（instance） | 同一个音效同时播放多次，每一次叫一个实例 |
| varispeed | 变速同时变调（像磁带快放），本项目的倍速方式 |
| SPSC | Single Producer Single Consumer，单写单读，环形缓冲的一种，不用加锁 |
| WASAPI / ALSA / PulseAudio / PipeWire | Windows / Linux 的系统音频接口 |
| WebView2 / WebKitGTK | Windows / Linux 上「在程序里显示网页」的系统组件 |
| Tauri | 用系统 WebView 做桌面程序界面的框架，比 Electron 小得多 |
| axum / tokio | Rust 的网页服务框架 / 异步运行时 |
| cpal | Rust 的跨平台音频库 |
| Vite | 前端打包工具 |
| CI | 持续集成：每次推送代码自动编译、检查、测试 |
| ADR | 架构决策记录，记录「为什么这样设计」（见 `docs/adr/`） |
| 反馈环 | 输出又被录回输入，越来越响，最后啸叫 |
| 回录设备 | 「立体声混音」这类设备，录的是电脑正在播放的所有声音 |

---

**接下来**：完成第 5 节的追踪后，打开 [代码走读](code-walkthrough.md)，它按模块列出了每个结构体和函数的用途，可以当字典查；想了解某个设计的来龙去脉，看 [音效板架构](soundboard-architecture.md) 和 [ADR](adr/)。
