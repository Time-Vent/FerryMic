// SoundBridge 前端：音效库网格 + 底部麦克风条 + 编辑/设置对话框。
// SoundBridge front-end: sound grid + bottom mic bar + edit / settings dialogs.
//
// 不依赖框架：状态放在模块变量里，列表变化时整体重绘网格，实时状态（播放中、电平）
// 只改对应元素，避免 100 ms 轮询导致整页闪动。
// No framework: state lives in module variables; the grid is re-rendered when the
// list changes, while live state (playing, level) only touches the affected
// elements so the 100 ms polling doesn't flicker the page.

import { api, type AppInfo, type DeviceItem, type MicStatus, type Sound, type TriggerMode } from './api';
import { icons } from './icons';

// ---- 状态 State ----

let sounds: Sound[] = [];
let playing = new Set<number>();
let query = '';
let mic: MicStatus | null = null;
let inputs: DeviceItem[] = [];

/** 实时状态轮询间隔（毫秒）。Live-state polling interval (ms). */
const LIVE_MS = 100;
/** 麦克风设备状态轮询间隔（毫秒）。Mic device status polling interval (ms). */
const MIC_STATUS_MS = 1000;

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

const modeLabel: Record<TriggerMode, string> = {
  OneShot: '点按',
  Loop: '循环',
  Exclusive: '独占',
  Overlap: '叠加',
};

const modeDesc: Record<TriggerMode, string> = {
  OneShot: '每按一次从头播放一遍，播完自动停止。',
  Loop: '按一下开始循环播放，再按一下停止。',
  Exclusive: '播放时打断其它所有音效，只留这一个。',
  Overlap: '可与其它音效同时播放，连按会叠加多份。',
};

// ---- 工具 Helpers ----

/** 把 data-icon 占位替换成 SVG。Replace data-icon placeholders with SVGs. */
function hydrateIcons(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>('[data-icon]').forEach((el) => {
    const name = el.dataset.icon as keyof typeof icons;
    if (icons[name] && !el.querySelector('svg')) el.insertAdjacentHTML('afterbegin', icons[name]);
  });
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** 右下角提示，几秒后自动消失。Bottom-right toast that fades after a few seconds. */
function toast(message: string, kind: 'info' | 'error' = 'info'): void {
  const t = el('div', `toast ${kind}`, message);
  $('toasts').appendChild(t);
  setTimeout(() => t.remove(), kind === 'error' ? 6000 : 3000);
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** 秒 → m:ss。Seconds → m:ss. */
function formatDuration(secs: number | null): string {
  if (secs == null) return '';
  const s = Math.max(0, Math.round(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** 快捷键字符串拆成按键显示名，如 "Ctrl+Alt+KeyQ" → ["Ctrl","Alt","Q"]。
 *  Split a hotkey string into display names, e.g. "Ctrl+Alt+KeyQ" → ["Ctrl","Alt","Q"]. */
function hotkeyParts(hotkey: string): string[] {
  const names: Record<string, string> = {
    ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
    Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
    Backslash: '\\', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/',
    Space: 'Space', Super: 'Win',
  };
  return hotkey.split('+').map((p) => {
    if (names[p]) return names[p];
    if (/^Key[A-Z]$/.test(p)) return p.slice(3);
    if (/^Digit\d$/.test(p)) return p.slice(5);
    if (/^Numpad/.test(p)) return `Num ${p.slice(6)}`;
    return p;
  });
}

function kbd(hotkey: string): HTMLElement {
  const wrap = el('span', 'kbd');
  for (const part of hotkeyParts(hotkey)) wrap.appendChild(el('kbd', '', part));
  return wrap;
}

// ---- 音效网格 Sound grid ----

async function loadSounds(): Promise<void> {
  try {
    sounds = await api.sounds();
    renderGrid();
  } catch (e) {
    toast(`无法读取音效库：${errorText(e)}`, 'error');
  }
}

function renderGrid(): void {
  const grid = $('grid');
  grid.innerHTML = '';
  const q = query.trim().toLowerCase();
  const visible = q
    ? sounds.filter((s) => s.name.toLowerCase().includes(q) || s.file_name.toLowerCase().includes(q))
    : sounds;
  $('sound-count').textContent = sounds.length ? `${sounds.length} 个音效` : '';

  if (sounds.length === 0) {
    grid.appendChild(renderEmpty());
    return;
  }
  if (visible.length === 0) {
    const empty = el('div', 'empty');
    empty.appendChild(el('p', '', `没有找到「${query}」`));
    grid.appendChild(empty);
    return;
  }
  for (const s of visible) grid.appendChild(renderPad(s));
}

function renderEmpty(): HTMLElement {
  const empty = el('div', 'empty');
  const icon = el('div', 'empty-icon');
  icon.innerHTML = icons.upload;
  empty.append(icon, el('h2', '', '还没有音效'), el('p', '', '把音频文件拖进窗口，或点击下方按钮导入'));
  const btn = el('button', 'btn primary');
  btn.innerHTML = `${icons.plus}<span>导入音效</span>`;
  btn.addEventListener('click', () => $<HTMLInputElement>('file-input').click());
  empty.appendChild(btn);
  return empty;
}

function renderPad(s: Sound): HTMLElement {
  const pad = el('article', 'pad');
  pad.dataset.id = String(s.id);
  pad.tabIndex = 0;
  if (s.error) pad.classList.add('broken');
  if (playing.has(s.id)) pad.classList.add('playing');

  // 圆形按钮：平时是播放，播放中是均衡器，悬停播放中的卡片变为停止。
  // Round button: play normally, equalizer while playing, stop when hovering a
  // playing pad.
  const play = el('span', 'pad-play');
  play.innerHTML = `<span class="i-play">${icons.play}</span><span class="i-eq eq"><i></i><i></i><i></i></span><span class="i-stop">${icons.stop}</span>`;
  play.title = '播放';
  play.addEventListener('click', (ev) => {
    if (pad.classList.contains('playing')) {
      ev.stopPropagation();
      api.stop(s.id).catch((e) => toast(errorText(e), 'error'));
    }
  });

  const body = el('div', 'pad-body');
  body.appendChild(el('div', 'pad-name', s.name));
  if (s.error) {
    const err = el('div', 'pad-error', s.error);
    err.title = s.error;
    body.appendChild(err);
  } else {
    const meta = el('div', 'pad-meta');
    meta.appendChild(el('span', 'tag mode', modeLabel[s.trigger_mode]));
    if (s.duration_secs != null) meta.appendChild(el('span', '', formatDuration(s.duration_secs)));
    if (s.speed !== 1) meta.appendChild(el('span', '', `${s.speed.toFixed(2)}×`));
    if (s.hotkey) meta.appendChild(kbd(s.hotkey));
    body.appendChild(meta);
  }

  const more = el('button', 'icon-btn pad-more');
  more.innerHTML = icons.more;
  more.title = '编辑';
  more.setAttribute('aria-label', `编辑 ${s.name}`);
  more.addEventListener('click', (ev) => {
    ev.stopPropagation();
    openEditor(s);
  });

  pad.append(play, body, more);
  const fire = () => {
    if (!s.error) api.trigger(s.id).catch((e) => toast(errorText(e), 'error'));
  };
  pad.addEventListener('click', fire);
  pad.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      fire();
    }
  });
  return pad;
}

/** 只更新播放中高亮，不重绘网格。Update playing highlight only, no grid re-render. */
function applyPlaying(): void {
  document.querySelectorAll<HTMLElement>('.pad').forEach((pad) => {
    pad.classList.toggle('playing', playing.has(Number(pad.dataset.id)));
  });
}

// ---- 导入 Import ----

async function importFiles(files: FileList | File[]): Promise<void> {
  const list = Array.from(files);
  if (list.length === 0) return;
  let ok = 0;
  for (const f of list) {
    try {
      await api.importSound(f);
      ok++;
    } catch (e) {
      toast(`「${f.name}」导入失败：${errorText(e)}`, 'error');
    }
  }
  if (ok) toast(ok === 1 ? '已导入 1 个音效' : `已导入 ${ok} 个音效`);
  await loadSounds();
}

function setupImport(): void {
  const input = $<HTMLInputElement>('file-input');
  $('import').addEventListener('click', () => input.click());
  input.addEventListener('change', () => {
    if (input.files) importFiles(input.files);
    input.value = '';
  });

  // 拖入文件：任意位置都能放下。Drag files anywhere onto the window.
  const overlay = $('drop-overlay');
  let depth = 0;
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');
  window.addEventListener('dragenter', (e) => {
    if (!hasFiles(e)) return;
    depth++;
    overlay.hidden = false;
  });
  window.addEventListener('dragleave', (e) => {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) overlay.hidden = true;
  });
  window.addEventListener('dragover', (e) => {
    if (hasFiles(e)) e.preventDefault();
  });
  window.addEventListener('drop', (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    overlay.hidden = true;
    if (e.dataTransfer?.files) importFiles(e.dataTransfer.files);
  });
}

// ---- 编辑对话框 Edit dialog ----

let editing: Sound | null = null;
let editMode: TriggerMode = 'OneShot';
let editHotkey: string | null = null;
let deleteArmed = false;

function setMode(mode: TriggerMode): void {
  editMode = mode;
  document.querySelectorAll<HTMLButtonElement>('#mode-group button').forEach((b) => {
    const on = b.dataset.mode === mode;
    b.classList.toggle('active', on);
    b.setAttribute('aria-checked', String(on));
  });
  $('mode-desc').textContent = modeDesc[mode];
}

function renderHotkeyButton(recording = false): void {
  const btn = $('hotkey-rec');
  btn.classList.toggle('recording', recording);
  btn.classList.toggle('has-key', !!editHotkey && !recording);
  btn.innerHTML = '';
  if (recording) btn.textContent = '请按下组合键…';
  else if (editHotkey) btn.appendChild(kbd(editHotkey));
  else btn.textContent = '点击设置快捷键';
}

/** 把按键事件转成后端格式（如 "Ctrl+Alt+KeyQ"）；只按了修饰键时返回 null。
 *  Convert a key event to the backend format (e.g. "Ctrl+Alt+KeyQ"); null when only
 *  modifiers are held. */
function hotkeyFromEvent(e: KeyboardEvent): string | null {
  if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/.test(e.code)) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');
  return [...mods, e.code].join('+');
}

function setupHotkeyRecorder(): void {
  const btn = $<HTMLButtonElement>('hotkey-rec');
  let recording = false;
  const stop = () => {
    recording = false;
    window.removeEventListener('keydown', onKey, true);
    renderHotkeyButton(false);
  };
  const onKey = (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === 'Escape') return stop();
    const hk = hotkeyFromEvent(e);
    if (!hk) return;
    const plain = !hk.includes('+');
    // 单独的字母/数字等会在全局拦截打字，要求带修饰键（功能键、小键盘除外）。
    // A bare letter/digit would hijack typing system-wide; require a modifier
    // (function keys and numpad excepted).
    if (plain && !/^(F\d{1,2}|Numpad.*|Pause|ScrollLock|Insert|Home|End|PageUp|PageDown)$/.test(hk)) {
      toast('单个字母或数字会影响打字，请加上 Ctrl / Alt / Shift', 'error');
      return;
    }
    editHotkey = hk;
    stop();
  };
  btn.addEventListener('click', () => {
    if (recording) return stop();
    recording = true;
    renderHotkeyButton(true);
    window.addEventListener('keydown', onKey, true);
  });
  $('hotkey-clear').addEventListener('click', () => {
    editHotkey = null;
    stop();
  });
  $<HTMLDialogElement>('edit-dialog').addEventListener('close', stop);
}

function openEditor(s: Sound): void {
  editing = s;
  deleteArmed = false;
  const form = $<HTMLFormElement>('edit-form');
  (form.elements.namedItem('name') as HTMLInputElement).value = s.name;
  const gain = form.elements.namedItem('gain_db') as HTMLInputElement;
  const speed = form.elements.namedItem('speed') as HTMLInputElement;
  gain.value = String(s.gain_db);
  speed.value = String(s.speed);
  updateSliderLabels();
  setMode(s.trigger_mode);
  editHotkey = s.hotkey;
  renderHotkeyButton(false);
  $('edit-file').textContent = s.error ? `文件：${s.file_name}（${s.error}）` : `文件：${s.file_name}`;
  $('edit-delete').querySelector('span:last-child')!.textContent = '删除';
  $<HTMLDialogElement>('edit-dialog').showModal();
}

function updateSliderLabels(): void {
  const form = $<HTMLFormElement>('edit-form');
  const gain = Number((form.elements.namedItem('gain_db') as HTMLInputElement).value);
  const speed = Number((form.elements.namedItem('speed') as HTMLInputElement).value);
  $('gain-out').textContent = `${gain > 0 ? '+' : ''}${gain} dB`;
  $('speed-out').textContent = `${speed.toFixed(2)}×`;
}

function setupEditor(): void {
  const dialog = $<HTMLDialogElement>('edit-dialog');
  const form = $<HTMLFormElement>('edit-form');
  document.querySelectorAll<HTMLButtonElement>('#mode-group button').forEach((b) =>
    b.addEventListener('click', () => setMode(b.dataset.mode as TriggerMode)),
  );
  form.addEventListener('input', updateSliderLabels);
  const speed = form.elements.namedItem('speed') as HTMLInputElement;
  speed.addEventListener('dblclick', () => {
    speed.value = '1';
    updateSliderLabels();
  });
  setupHotkeyRecorder();

  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    if (!editing) return;
    const data = new FormData(form);
    try {
      await api.updateSound(editing.id, {
        name: String(data.get('name') ?? '').trim(),
        trigger_mode: editMode,
        gain_db: Number(data.get('gain_db')),
        speed: Number(data.get('speed')),
        hotkey: editHotkey,
      });
      dialog.close();
      await loadSounds();
    } catch (e) {
      toast(`保存失败：${errorText(e)}`, 'error');
    }
  });

  // 删除需要点两次确认。Delete needs a second click to confirm.
  $('edit-delete').addEventListener('click', async () => {
    if (!editing) return;
    const label = $('edit-delete').querySelector('span:last-child')!;
    if (!deleteArmed) {
      deleteArmed = true;
      label.textContent = '再点一次确认删除';
      return;
    }
    try {
      await api.removeSound(editing.id);
      dialog.close();
      toast(`已删除「${editing.name}」`);
      await loadSounds();
    } catch (e) {
      toast(`删除失败：${errorText(e)}`, 'error');
    }
  });
}

// ---- 麦克风条 Mic bar ----

function renderMic(m: MicStatus): void {
  mic = m;
  const bar = document.querySelector('.mixer')!;
  bar.classList.toggle('muted', m.muted);
  const toggle = $('mic-mute');
  toggle.innerHTML = m.muted || !m.connected ? icons.micOff : icons.mic;
  toggle.classList.toggle('muted', m.muted);
  toggle.classList.toggle('offline', !m.connected && !m.muted);
  toggle.title = m.muted ? '已静音，点击取消' : '点击静音';
  $('mic-name').textContent = m.connected ? (m.device ?? '麦克风') : '麦克风未连接';
  const hint = $('mic-hint');
  hint.textContent = m.connected ? (m.muted ? '已静音' : `${m.sample_rate ?? ''} Hz`) : (m.note ?? '等待麦克风连接…');
  hint.classList.toggle('warn', !m.connected);
  hint.title = hint.textContent ?? '';
  const gain = $<HTMLInputElement>('mic-gain');
  if (document.activeElement !== gain) {
    gain.value = String(m.gain_db);
    $('mic-gain-value').textContent = `${m.gain_db > 0 ? '+' : ''}${m.gain_db} dB`;
  }
  renderMicStatusInSettings();
}

/** 电平 → 百分比（dB 刻度，-60 dB = 0%）。Level → percent (dB scale, -60 dB = 0%). */
function levelPercent(level: number): number {
  const db = level > 0 ? 20 * Math.log10(level) : -60;
  return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
}

function setupMicBar(): void {
  $('mic-mute').addEventListener('click', async () => {
    if (!mic) return;
    try {
      renderMic(await api.setMic({ muted: !mic.muted }));
    } catch (e) {
      toast(errorText(e), 'error');
    }
  });
  const gain = $<HTMLInputElement>('mic-gain');
  let timer = 0;
  gain.addEventListener('input', () => {
    const v = Number(gain.value);
    $('mic-gain-value').textContent = `${v > 0 ? '+' : ''}${v} dB`;
    clearTimeout(timer);
    timer = window.setTimeout(() => api.setMic({ gain_db: v }).catch(() => {}), 120);
  });
  gain.addEventListener('dblclick', () => {
    gain.value = '0';
    gain.dispatchEvent(new Event('input'));
  });
}

// ---- 设置 Settings ----

function renderMicSelect(): void {
  const select = $<HTMLSelectElement>('mic-select');
  if (document.activeElement === select) return;
  const selection = mic?.selection ?? null;
  const defaultName = inputs.find((d) => d.is_default)?.name;
  select.innerHTML = '';
  select.appendChild(new Option(defaultName ? `跟随系统默认（${defaultName}）` : '跟随系统默认', ''));
  for (const d of inputs) select.appendChild(new Option(d.name, d.name));
  // 选中的设备暂时不在（如蓝牙已断开）：仍列出。Chosen device absent: still list it.
  if (selection && !inputs.some((d) => d.name === selection)) {
    select.appendChild(new Option(`${selection}（未连接）`, selection));
  }
  select.value = selection ?? '';
}

function renderMicStatusInSettings(): void {
  if (!mic) return;
  $('mic-status').textContent = mic.connected
    ? `正在使用：${mic.device}（${mic.sample_rate} Hz）`
    : (mic.note ?? '等待麦克风连接…');
}

function renderAppInfo(info: AppInfo): void {
  $('output-name').textContent = info.output.output_device;
  $('settings-output').textContent = `${info.output.output_device}（${info.output.output_rate} Hz）`;
  $('app-version').textContent = `v${info.version}`;
  $('data-dir').textContent = info.data_dir;
  $<HTMLInputElement>('lan-toggle').checked = info.lan_remote;
  const url = $('lan-url');
  url.innerHTML = '';
  if (info.lan_active && info.lan_url) {
    url.append('手机访问：');
    const a = el('a', '', info.lan_url);
    a.href = info.lan_url;
    a.target = '_blank';
    url.appendChild(a);
  }
  const note = $('lan-note');
  if (info.lan_remote !== info.lan_active) note.textContent = '重启 SoundBridge 后生效。';
  else if (info.lan_active) note.textContent = '手机和电脑需在同一 Wi-Fi；首次开启时 Windows 可能询问防火墙权限。';
  else note.textContent = '关闭时只允许本机访问。';
}

async function loadDevices(): Promise<void> {
  try {
    inputs = (await api.devices()).inputs;
    renderMicSelect();
  } catch (e) {
    toast(`无法读取设备列表：${errorText(e)}`, 'error');
  }
}

function setupSettings(): void {
  const dialog = $<HTMLDialogElement>('settings-dialog');
  const open = async () => {
    dialog.showModal();
    document.getElementById('open-settings')?.classList.add('active');
    loadDevices();
    try {
      renderAppInfo(await api.app());
    } catch (e) {
      toast(errorText(e), 'error');
    }
  };
  $('open-settings').addEventListener('click', open);
  $('open-settings-mobile').addEventListener('click', open);
  dialog.addEventListener('close', () => $('open-settings').classList.remove('active'));

  $('mic-refresh').addEventListener('click', loadDevices);
  const select = $<HTMLSelectElement>('mic-select');
  select.addEventListener('change', async () => {
    select.disabled = true;
    try {
      renderMic(await api.selectMic(select.value || null));
    } catch (e) {
      toast(`切换麦克风失败：${errorText(e)}`, 'error');
    } finally {
      select.disabled = false;
      select.blur();
      renderMicSelect();
    }
  });

  $('lan-toggle').addEventListener('change', async (ev) => {
    try {
      renderAppInfo(await api.setSettings({ lan_remote: (ev.target as HTMLInputElement).checked }));
    } catch (e) {
      toast(errorText(e), 'error');
    }
  });
}

// ---- 通用对话框行为 Common dialog behaviour ----

function setupDialogs(): void {
  document.querySelectorAll<HTMLDialogElement>('dialog').forEach((d) => {
    // 点遮罩关闭。Click the backdrop to close.
    d.addEventListener('click', (e) => {
      if (e.target === d) d.close();
    });
    d.querySelectorAll('[data-close]').forEach((b) => b.addEventListener('click', () => d.close()));
  });
}

// ---- 轮询 Polling ----

function startPolling(): void {
  const live = async () => {
    try {
      const s = await api.live();
      const next = new Set(s.playing);
      if (next.size !== playing.size || [...next].some((id) => !playing.has(id))) {
        playing = next;
        applyPlaying();
      }
      $('mic-meter').style.width = `${levelPercent(s.mic_level)}%`;
    } catch {
      /* 后端暂时不可用：下次再试。Backend briefly unavailable: retry next tick. */
    }
    setTimeout(live, LIVE_MS);
  };
  live();

  const micStatus = async () => {
    try {
      const m = await api.mic();
      const changed = m.selection !== mic?.selection;
      renderMic(m);
      if (changed) renderMicSelect();
    } catch {
      /* 同上。Same as above. */
    }
    setTimeout(micStatus, MIC_STATUS_MS);
  };
  micStatus();
}

// ---- 滑块填充 Slider fill ----

/** 按滑块当前值更新已填充比例（CSS 变量 --fill）。Update a slider's filled ratio (CSS var --fill). */
function syncRangeFill(input: HTMLInputElement): void {
  const min = Number(input.min || 0);
  const max = Number(input.max || 100);
  const pct = ((Number(input.value) - min) / (max - min)) * 100;
  input.style.setProperty('--fill', `${Math.max(0, Math.min(100, pct))}%`);
}

/** 所有滑块：用户拖动或脚本赋值后都同步填充。All sliders: sync on drag and on scripted updates. */
function setupRangeFill(): void {
  const ranges = document.querySelectorAll<HTMLInputElement>('input[type="range"]');
  ranges.forEach((r) => {
    syncRangeFill(r);
    r.addEventListener('input', () => syncRangeFill(r));
  });
  // 脚本直接改 value 不触发 input 事件，定时兜底同步。
  // Scripted value changes don't fire input events; resync periodically.
  setInterval(() => ranges.forEach(syncRangeFill), 250);
}

// ---- 启动 Start ----

hydrateIcons();
setupRangeFill();
setupDialogs();
setupImport();
setupEditor();
setupMicBar();
setupSettings();
$<HTMLInputElement>('search').addEventListener('input', (e) => {
  query = (e.target as HTMLInputElement).value;
  renderGrid();
});
$('stop-all').addEventListener('click', () => api.stopAll().catch((e) => toast(errorText(e), 'error')));
api
  .app()
  .then(renderAppInfo)
  .catch(() => {});
loadSounds();
startPolling();
