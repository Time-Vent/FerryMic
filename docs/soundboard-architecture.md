# 音效板架构（Soundboard）

> 本文档描述「音效板」功能的技术架构：多音效管理、快捷键触发、Web 界面、手机遥控。

## 1. 定位

音效板是 SoundBridge 的**核心产品形态**：管理多个音效，绑定快捷键，随时把音效混入
麦克风输出到声卡/虚拟音频线。典型场景：游戏语音整活、会议、直播。

## 2. 模块架构

```
apps/server（主程序 soundbridge-server）
  ├─ main.rs         入口 + axum 路由 + 共享状态 AppState
  ├─ audio.rs        音频线程（SoundBoard + cpal 输出）
  ├─ hotkeys.rs      全局热键线程（global-hotkey）
  └─ （依赖）soundboard crate   音效板核心
        └─ （依赖）audio-source  VarispeedSource（文件解码 + 变速）
```

```
crates/soundboard/
  ├─ TriggerMode    触发模式：OneShot / Loop / Exclusive / Overlap
  ├─ SoundSpec      音效定义：名称/文件/模式/增益/变速/快捷键
  ├─ BoardCommand   控制命令：AddSound / Trigger / Stop / SetGain / SetSpeed ...
  ├─ Instance       播放实例（一个 VarispeedSource + 播放状态）
  └─ SoundBoard     音效板：管理定义与实例，fill 混音（实时线程独占）
```

## 3. 线程拓扑

| 线程 | 职责 | 实时约束 |
|---|---|---|
| tokio 主线程 | axum 服务器，REST API | 无 |
| cpal 输出回调线程 | 消费命令(≤32) → SoundBoard::fill → 写输出 | **实时**：不分配/不加锁 |
| 热键线程 | global-hotkey 事件循环，热键按下发 Trigger | 无（非实时） |

**命令流**：

```
Web API ──cmd_tx──▶ 音频回调（SoundBoard.apply_command）
全局热键 ──Trigger──▶ 音频回调
```

## 4. 触发语义

| 模式 | 行为 |
|---|---|
| `OneShot`（点按） | 每按一次从头播一遍，播完自动停止 |
| `Loop`（循环） | 按一下开始循环，再按停止 |
| `Exclusive`（独占） | 播放时停止其它所有音效 |
| `Overlap`（叠加） | 可与其它音效同时播放 |

每个音效独立配置：触发模式 + 增益（dB）+ 倍速（varispeed，变速变调）。

## 5. 混音（SoundBoard::fill）

- 遍历所有播放实例，每个实例 `VarispeedSource::fill` 到 scratch，按增益累加到输出。
- 实例源声道转换到输出声道（mono 复制 / stereo 直接 / 其它取交集）。
- 非循环实例播放完（exhausted）自动移除；循环实例 reset 继续。
- **不分配、不加锁**：scratch 预分配，实例列表 `Vec` 原地操作（`swap_remove`）。

## 6. 全局热键

- 依赖 `global-hotkey`（Windows RegisterHotKey / macOS / Linux X11）。
- 快捷键字符串由 `HotKey::from_str` 解析（如 `F1`、`Ctrl+Alt+1`）。
- 热键 id = `mods.bits() << 16 | key`，事件通过 `GlobalHotKeyEvent::receiver()` 接收。
- 热键线程维护 `hotkey_id → sound_id` 映射，按下时发 `Trigger` 命令。

## 7. 手机遥控

手机**不做音频处理**，只当「遥控器」：

```
手机浏览器 ──HTTP/局域网──▶ 电脑 Web 服务 ──命令──▶ 音频引擎 ──▶ 声卡/虚拟音频线
```

- 手机浏览器打开 `http://<电脑IP>:8321`，加载同一套 Web 前端。
- 手机发 `POST /api/sounds/:id/trigger` 等请求，音频在电脑处理输出。
- 绕开「移动端不能系统级注入麦克风」的平台限制。

## 8. 输出设备选择

- `GET /api/devices`：列出可用输出设备（声卡 / 虚拟音频线）。
- 启动时通过环境变量 `SOUNDBRIDGE_OUTPUT_DEVICE` 指定（模糊匹配设备名）。
- `CpalBackend::open_output_named` 按名称匹配设备，否则用默认输出设备。
- 运行时切换输出设备（重建流）留待后续（涉及实时线程安全，见 ADR-0005）。

## 9. 数据模型（前端 JSON）

```jsonc
{
  "id": 1,
  "name": "鼓掌",
  "file_path": "C:\\sounds\\clap.wav",
  "trigger_mode": "OneShot",   // OneShot | Loop | Exclusive | Overlap
  "gain_db": 0.0,
  "speed": 1.0,
  "hotkey": "F1"                // 可选，如 "Ctrl+Alt+1"
}
```
