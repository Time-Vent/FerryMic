// FerryMic 前端：文件夹栏 + 音效卡片网格 + 底部麦克风条 + 弹出层 / 对话框 / 设置。
// FerryMic front-end: folder sidebar + sound grid + bottom mic bar + popovers /
// dialogs / settings.
//
// 不依赖框架：状态放在模块变量里，音效库变化时重绘文件夹栏与网格；实时状态（播放中、
// 进度、电平）只改对应元素，避免 100 ms 轮询导致整页闪动。
// No framework: state lives in module variables; library changes re-render the sidebar
// and grid, while live state (playing, progress, level) only touches the affected
// elements so the 100 ms polling doesn't flicker the page.

import {
  api,
  type AppInfo,
  type CloseAction,
  type InputItem,
  type Library,
  type MicStatus,
  type MonitorStatus,
  type OutputInfo,
  type OutputItem,
  type RemoteStatus,
  type Sound,
  type CustomSite,
  type SoundDefaults,
  type SoundPatch,
  type ThemeMode,
} from './api';
import { icons } from './icons';
import { BUILTIN_SITES, hostOf, normalizeUrl } from './sites';
import { currentLang, initLang, setLang, t, type Lang, type LangPref } from './i18n';

// ---- 状态 State ----

let library: Library = { root: '', folders: [], sounds: [] };
/** 当前文件夹（'' = 全部音效）。Current folder ('' = all sounds). */
let currentFolder = '';
/** 播放中的音效 → 进度 0..1。Playing sounds → progress 0..1. */
let playing = new Map<number, number>();
/** 正在拖动进度条的音效（期间不用轮询结果覆盖）。Sounds whose progress bar is being dragged. */
const seeking = new Set<number>();
let query = '';
let mic: MicStatus | null = null;
let inputs: InputItem[] = [];
let outputs: OutputItem[] = [];
let monitor: MonitorStatus | null = null;
let appInfo: AppInfo | null = null;
/** 折叠的文件夹（本机记住）。Collapsed folders (remembered locally). */
const collapsed = new Set<string>(readLocal<string[]>('sb-collapsed') ?? []);

/** 实时状态轮询间隔（毫秒）。Live-state polling interval (ms). */
const LIVE_MS = 100;
/** 麦克风设备状态轮询间隔（毫秒）。Mic device status polling interval (ms). */
const MIC_STATUS_MS = 1000;
/** 拖动卡片时的数据类型。Data type used when dragging a pad. */
const SOUND_MIME = 'application/x-soundbridge-sound';

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;


/** 预设主题色。Preset accent colours. */
const ACCENTS: { name: string; color: string }[] = [
  { name: '紫', color: '#7b6cff' },
  { name: '蓝', color: '#3b82f6' },
  { name: '青', color: '#14b8a6' },
  { name: '绿', color: '#22c55e' },
  { name: '橙', color: '#f59e0b' },
  { name: '粉', color: '#ec4899' },
  { name: '红', color: '#ef4444' },
];

const SPEED_PRESETS = [0.5, 0.75, 1, 1.25, 1.5, 2];

// ---- 工具 Helpers ----

function readLocal<T>(key: string): T | null {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null') as T | null;
  } catch {
    return null;
  }
}

function writeLocal(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* 存储不可用时忽略。Ignore when storage is unavailable. */
  }
}

/** 把 data-icon 占位替换成 SVG。Replace data-icon placeholders with SVGs. */
function hydrateIcons(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>('[data-icon]').forEach((e) => {
    const name = e.dataset.icon as keyof typeof icons;
    if (icons[name] && !e.querySelector(':scope > svg.icon')) e.insertAdjacentHTML('afterbegin', icons[name]);
  });
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** 按模板追加文字与节点：`{名字}` 处放入对应节点（用于句中带 <code> 的翻译）。
 *  Append text and nodes from a template: `{name}` slots take the given nodes (for
 *  translated sentences containing <code>). */
function appendTemplate(parent: HTMLElement, template: string, nodes: Record<string, Node>): void {
  for (const part of template.split(/(\{\w+\})/)) {
    const key = /^\{(\w+)\}$/.exec(part)?.[1];
    if (key && nodes[key]) parent.append(nodes[key]);
    else if (part) parent.append(part);
  }
}

/** 带图标的按钮。A button with an icon. */
function iconButton(cls: string, icon: keyof typeof icons, label: string): HTMLButtonElement {
  const b = el('button', cls);
  b.type = 'button';
  b.innerHTML = icons[icon];
  b.title = label;
  b.setAttribute('aria-label', label);
  return b;
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
  if (secs == null || !isFinite(secs)) return '';
  const s = Math.max(0, Math.round(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

function formatGain(db: number): string {
  return `${db > 0 ? '+' : ''}${Math.round(db)} dB`;
}

function formatSpeed(speed: number): string {
  return `${speed.toFixed(2).replace(/0$/, '')}×`;
}

/** 快捷键字符串拆成按键显示名，如 "Ctrl+Alt+KeyQ" → ["Ctrl","Alt","Q"]。
 *  Split a hotkey string into display names, e.g. "Ctrl+Alt+KeyQ" → ["Ctrl","Alt","Q"]. */
function hotkeyParts(hotkey: string): string[] {
  const names: Record<string, string> = {
    ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
    Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
    Backslash: '\\', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/',
    Space: 'Space', Super: 'Win',
    MouseMiddle: 'Mouse 3', Mouse4: 'Mouse 4', Mouse5: 'Mouse 5',
  };
  return hotkey.split('+').map((p) => {
    if (names[p]) return names[p];
    if (/^Key[A-Z]$/.test(p)) return p.slice(3);
    if (/^Digit\d$/.test(p)) return p.slice(5);
    if (/^Numpad/.test(p)) return `Num ${p.slice(6)}`;
    return p;
  });
}

function kbd(hotkey: string, passthrough = false): HTMLElement {
  const wrap = el('span', passthrough ? 'kbd passthrough' : 'kbd');
  if (passthrough) wrap.title = t('不拦截：按键照样传给游戏');
  for (const part of hotkeyParts(hotkey)) wrap.appendChild(el('kbd', '', part));
  return wrap;
}

// ---- 主题 Theme ----

function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function rgbToHsl([r, g, b]: [number, number, number]): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = 0;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

function hslToHex(h: number, s: number, l: number): string {
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const a = s * Math.min(l, 1 - l);
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255).toString(16).padStart(2, '0');
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

/** 由主题色算出整套强调色变量。Derive the full set of accent variables from one colour. */
function accentVars(accent: string): Record<string, string> {
  const [r, g, b] = hexToRgb(accent);
  const [h, s, l] = rgbToHsl([r, g, b]);
  // 渐变的第二色：色相偏移 +35°、略亮。Second gradient colour: hue +35°, a bit lighter.
  const second = hslToHex((h + 35) % 360, Math.min(1, s * 0.95), Math.min(0.72, l + 0.08));
  return {
    '--accent': accent,
    '--accent-2': second,
    '--accent-soft': `rgba(${r}, ${g}, ${b}, 0.16)`,
    '--accent-glow': `rgba(${r}, ${g}, ${b}, 0.3)`,
    '--accent-line': `rgba(${r}, ${g}, ${b}, 0.55)`,
  };
}

/** 套用外观并记到本机（下次启动先用它，避免闪烁）。Apply the theme and remember it locally. */
function applyTheme(mode: ThemeMode, accent: string): void {
  const root = document.documentElement;
  if (mode === 'system') delete root.dataset.theme;
  else root.dataset.theme = mode;
  const vars = accentVars(accent);
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v);
  writeLocal('sb-theme', { mode, vars });
  renderThemeControls(mode, accent);
}

function renderThemeControls(mode: ThemeMode, accent: string): void {
  document.querySelectorAll<HTMLButtonElement>('[data-theme-mode]').forEach((b) =>
    b.classList.toggle('active', b.dataset.themeMode === mode),
  );
  document.querySelectorAll<HTMLElement>('.swatch[data-color]').forEach((s) =>
    s.classList.toggle('active', s.dataset.color === accent.toLowerCase()),
  );
  const custom = document.querySelector<HTMLElement>('.swatch.custom')!;
  custom.classList.toggle('active', !ACCENTS.some((a) => a.color === accent.toLowerCase()));
  $<HTMLInputElement>('accent-custom').value = accent;
}

async function saveTheme(patch: { theme_mode?: ThemeMode; accent?: string }): Promise<void> {
  if (!appInfo) return;
  const mode = patch.theme_mode ?? appInfo.theme_mode;
  const accent = patch.accent ?? appInfo.accent;
  applyTheme(mode, accent);
  try {
    appInfo = await api.setSettings(patch);
  } catch (e) {
    toast(t('保存外观失败：{error}', { error: errorText(e) }), 'error');
  }
}

function setupTheme(): void {
  const swatches = $('accent-swatches');
  for (const a of ACCENTS) {
    const s = el('button', 'swatch');
    s.type = 'button';
    s.dataset.color = a.color;
    s.style.background = a.color;
    s.title = a.name;
    s.addEventListener('click', () => saveTheme({ accent: a.color }));
    swatches.insertBefore(s, swatches.querySelector('.custom'));
  }
  const custom = $<HTMLInputElement>('accent-custom');
  // 拖动取色时实时预览，松开后保存。Live preview while picking, save on change.
  custom.addEventListener('input', () => applyTheme(appInfo?.theme_mode ?? 'system', custom.value));
  custom.addEventListener('change', () => saveTheme({ accent: custom.value }));
  document.querySelectorAll<HTMLButtonElement>('[data-theme-mode]').forEach((b) =>
    b.addEventListener('click', () => saveTheme({ theme_mode: b.dataset.themeMode as ThemeMode })),
  );
}

// ---- 音效库 Library ----

async function loadLibrary(): Promise<void> {
  try {
    applyLibrary(await api.library());
  } catch (e) {
    toast(t('无法读取音效库：{error}', { error: errorText(e) }), 'error');
  }
}

function applyLibrary(lib: Library): void {
  library = lib;
  // 当前文件夹被删除 / 改名后回到「全部」。Fall back to "all" if the folder vanished.
  if (currentFolder && !lib.folders.includes(currentFolder)) currentFolder = '';
  $('library-path').textContent = lib.root;
  $('library-path').parentElement!.title = t('在文件管理器中打开：{path}', { path: lib.root });
  renderSidebar();
  renderChips();
  renderGrid();
}

/** 文件夹的最后一段名称。The last segment of a folder path. */
function folderName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

function parentOf(path: string): string {
  const i = path.lastIndexOf('/');
  return i < 0 ? '' : path.slice(0, i);
}

function inFolder(sound: Sound, folder: string): boolean {
  return !folder || sound.folder === folder || sound.folder.startsWith(`${folder}/`);
}

/** 每个文件夹（含子文件夹）的音效数。Sound count per folder (incl. subfolders). */
function folderCounts(): Map<string, number> {
  const counts = new Map<string, number>();
  for (const s of library.sounds) {
    let f = s.folder;
    while (f) {
      counts.set(f, (counts.get(f) ?? 0) + 1);
      f = parentOf(f);
    }
  }
  return counts;
}

function selectFolder(path: string): void {
  currentFolder = path;
  renderSidebar();
  renderChips();
  renderGrid();
}

// ---- 文件夹栏 Folder sidebar ----

function renderSidebar(): void {
  const tree = $('folder-tree');
  tree.innerHTML = '';
  const counts = folderCounts();
  const children = new Map<string, string[]>();
  for (const f of library.folders) {
    const p = parentOf(f);
    children.set(p, [...(children.get(p) ?? []), f]);
  }

  tree.appendChild(treeItem('', t('全部音效'), 0, false, library.sounds.length, true));
  const walk = (parent: string, depth: number) => {
    for (const f of children.get(parent) ?? []) {
      const hasKids = (children.get(f)?.length ?? 0) > 0;
      tree.appendChild(treeItem(f, folderName(f), depth, hasKids, counts.get(f) ?? 0, false));
      if (hasKids && !collapsed.has(f)) walk(f, depth + 1);
    }
  };
  walk('', 1);
}

function treeItem(path: string, name: string, depth: number, hasKids: boolean, count: number, isRoot: boolean): HTMLElement {
  const item = el('div', 'tree-item');
  item.setAttribute('role', 'treeitem');
  item.style.paddingLeft = `${6 + Math.max(0, depth - 1) * 14}px`;
  if (path === currentFolder) item.classList.add('active');

  if (!isRoot) {
    const toggle = el('button', `tree-toggle${hasKids ? '' : ' empty'}${collapsed.has(path) ? '' : ' open'}`);
    toggle.type = 'button';
    toggle.innerHTML = icons.chevron;
    toggle.addEventListener('click', (e) => {
      e.stopPropagation();
      if (collapsed.has(path)) collapsed.delete(path);
      else collapsed.add(path);
      writeLocal('sb-collapsed', [...collapsed]);
      renderSidebar();
    });
    item.appendChild(toggle);
  }
  const icon = el('span', 'tree-icon');
  icon.innerHTML = isRoot ? icons.library : icons.folder;
  item.append(icon, el('span', 'tree-name', name), el('span', 'tree-count', String(count)));

  if (!isRoot) {
    const more = iconButton('icon-btn tree-more', 'more', t('「{name}」更多操作', { name }));
    more.addEventListener('click', (e) => {
      e.stopPropagation();
      openFolderMenu(path, more);
    });
    item.appendChild(more);
  }
  item.addEventListener('click', () => selectFolder(path));
  setupDropTarget(item, path);
  return item;
}

/** 文件夹作为放置目标：拖入卡片 = 移动，拖入文件 = 导入到这里。
 *  A folder as a drop target: drop a pad = move it, drop files = import here. */
function setupDropTarget(target: HTMLElement, folder: string): void {
  target.addEventListener('dragover', (e) => {
    const types = Array.from(e.dataTransfer?.types ?? []);
    if (!types.includes(SOUND_MIME) && !types.includes('Files')) return;
    e.preventDefault();
    target.classList.add('drop-target');
    if (types.includes('Files')) $('drop-target').textContent = t('松开即可导入到「{folder}」', { folder: folder ? folderName(folder) : t('全部音效') });
  });
  target.addEventListener('dragleave', () => {
    target.classList.remove('drop-target');
    $('drop-target').textContent = dropText();
  });
  target.addEventListener('drop', async (e) => {
    target.classList.remove('drop-target');
    const id = e.dataTransfer?.getData(SOUND_MIME);
    if (id) {
      e.preventDefault();
      e.stopPropagation();
      await patchSound(Number(id), { folder }, true);
      return;
    }
    if (e.dataTransfer?.files?.length) {
      e.preventDefault();
      e.stopPropagation();
      hideDropOverlay();
      importFiles(e.dataTransfer.files, folder);
    }
  });
}

function openFolderMenu(path: string, anchor: HTMLElement): void {
  const menu = el('div', 'menu');
  const add = (icon: keyof typeof icons, label: string, fn: () => void, danger = false) => {
    const b = el('button', danger ? 'danger' : '');
    b.type = 'button';
    b.innerHTML = `${icons[icon]}<span>${label}</span>`;
    b.addEventListener('click', () => {
      closePopover();
      fn();
    });
    menu.appendChild(b);
  };
  add('folderPlus', t('新建子文件夹'), () => newFolder(path));
  add('folder', t('重命名'), () => renameFolder(path));
  add('trash', t('删除'), () => deleteFolder(path), true);
  openPopover(anchor, menu);
}

async function newFolder(parent: string): Promise<void> {
  const name = await promptText({ title: t('新建文件夹'), message: parent ? t('在「{parent}」里新建', { parent }) : t('在音效库根目录新建'), value: '' });
  if (!name) return;
  try {
    const path = parent ? `${parent}/${name}` : name;
    applyLibrary(await api.createFolder(path));
    collapsed.delete(parent);
    selectFolder(library.folders.find((f) => f.endsWith(name)) ?? currentFolder);
  } catch (e) {
    toast(t('新建失败：{error}', { error: errorText(e) }), 'error');
  }
}

async function renameFolder(path: string): Promise<void> {
  const name = await promptText({ title: t('重命名文件夹'), message: t('当前：{path}', { path }), value: folderName(path) });
  if (!name || name === folderName(path)) return;
  const parent = parentOf(path);
  const to = parent ? `${parent}/${name}` : name;
  try {
    const wasCurrent = currentFolder === path;
    applyLibrary(await api.renameFolder(path, to));
    if (wasCurrent) selectFolder(to);
  } catch (e) {
    toast(t('重命名失败：{error}', { error: errorText(e) }), 'error');
  }
}

async function deleteFolder(path: string): Promise<void> {
  const ok = await promptText({
    title: t('删除文件夹'),
    message: t('确定删除「{path}」？只能删除没有音效的文件夹，删除的文件夹会移到回收站。', { path }),
    okText: t('删除'),
    confirmOnly: true,
  });
  if (ok === null) return;
  try {
    applyLibrary(await api.deleteFolder(path));
  } catch (e) {
    toast(t('删除失败：{error}', { error: errorText(e) }), 'error');
  }
}

/** 窄屏的文件夹切换条。Folder chips on narrow screens. */
function renderChips(): void {
  const bar = $('folder-chips');
  bar.innerHTML = '';
  const add = (path: string, label: string) => {
    const c = el('button', `chip${path === currentFolder ? ' active' : ''}`, label);
    c.type = 'button';
    c.addEventListener('click', () => selectFolder(path));
    bar.appendChild(c);
  };
  add('', t('全部'));
  for (const f of library.folders) add(f, f);
}

// ---- 音效网格 Sound grid ----

function renderGrid(): void {
  const grid = $('grid');
  grid.innerHTML = '';
  const q = query.trim().toLowerCase();
  const inView = library.sounds.filter((s) => inFolder(s, currentFolder));
  const visible = q
    ? inView.filter((s) => s.name.toLowerCase().includes(q) || s.file_name.toLowerCase().includes(q))
    : inView;
  $('view-title').textContent = currentFolder ? folderName(currentFolder) : t('全部音效');
  $('view-title').title = currentFolder || library.root;
  $('sound-count').textContent = inView.length ? t('{n} 个音效', { n: inView.length }) : '';

  if (inView.length === 0) {
    grid.appendChild(renderEmpty());
    return;
  }
  if (visible.length === 0) {
    const empty = el('div', 'empty');
    empty.appendChild(el('p', '', t('没有找到「{query}」', { query })));
    grid.appendChild(empty);
    return;
  }
  for (const s of visible) grid.appendChild(renderPad(s));
}

function renderEmpty(): HTMLElement {
  const empty = el('div', 'empty');
  const icon = el('div', 'empty-icon');
  icon.innerHTML = icons.upload;
  const where = currentFolder ? t('「{folder}」还没有音效', { folder: folderName(currentFolder) }) : t('还没有音效');
  empty.append(icon, el('h2', '', where), el('p', '', t('把音频文件拖进窗口，或点击下方按钮导入')));
  const btn = el('button', 'btn primary');
  btn.innerHTML = `${icons.plus}<span>${t('导入音效')}</span>`;
  btn.addEventListener('click', () => $<HTMLInputElement>('file-input').click());
  empty.appendChild(btn);
  return empty;
}

function renderPad(s: Sound): HTMLElement {
  const pad = el('article', 'pad');
  pad.dataset.id = String(s.id);
  pad.tabIndex = 0;
  pad.draggable = true;
  if (s.error) pad.classList.add('broken');
  if (playing.has(s.id)) pad.classList.add('playing');

  // 上半部分：圆形按钮 + 名称 / 信息。Top: round button + name / info.
  const top = el('div', 'pad-top');
  const play = el('span', 'pad-play');
  play.innerHTML = `<span class="i-play">${icons.play}</span><span class="i-eq eq"><i></i><i></i><i></i></span><span class="i-stop">${icons.stop}</span>`;
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
    const time = el('span', 'pad-time', formatDuration(s.duration_secs));
    meta.appendChild(time);
    if (s.hotkey) meta.appendChild(kbd(s.hotkey, s.hotkey_passthrough));
    // 「全部」视图里显示所在文件夹。Show the folder in the "all" view.
    if (s.folder && s.folder !== currentFolder) {
      const tag = el('span', 'tag folder-tag', folderName(s.folder));
      tag.title = s.folder;
      meta.appendChild(tag);
    }
    body.appendChild(meta);
  }
  const more = iconButton('icon-btn pad-more', 'more', t('编辑 {name}', { name: s.name }));
  more.addEventListener('click', (ev) => {
    ev.stopPropagation();
    openEditor(s);
  });
  top.append(play, body, more);
  pad.appendChild(top);

  if (!s.error) {
    pad.appendChild(renderProgress(s));
    pad.appendChild(renderControls(s));
  }

  // 点卡片播放；Enter / 空格同样。Click to play; Enter / Space too.
  const fire = () => {
    if (!s.error) api.trigger(s.id).catch((e) => toast(errorText(e), 'error'));
  };
  pad.addEventListener('click', fire);
  pad.addEventListener('keydown', (ev) => {
    if (ev.target === pad && (ev.key === 'Enter' || ev.key === ' ')) {
      ev.preventDefault();
      fire();
    }
  });

  // 拖到左侧文件夹上即可移动。Drag onto a folder on the left to move it.
  pad.addEventListener('dragstart', (ev) => {
    // 正在拖进度条时不拖卡片。Don't drag the pad while scrubbing its progress bar.
    if (seeking.has(s.id)) {
      ev.preventDefault();
      return;
    }
    ev.dataTransfer?.setData(SOUND_MIME, String(s.id));
    if (ev.dataTransfer) ev.dataTransfer.effectAllowed = 'move';
    pad.classList.add('dragging');
  });
  pad.addEventListener('dragend', () => pad.classList.remove('dragging'));
  return pad;
}

/** 底部快捷控件：循环 / 单次（图标）、独占 / 叠加、倍速（点击切换）、音量（点击弹出竖向滑条）。
 *  Bottom quick controls: loop / once (icon), exclusive / overlap and speed (click to
 *  toggle / cycle), volume (click for a vertical slider). */
function renderControls(s: Sound): HTMLElement {
  const row = el('div', 'pad-controls');

  // 循环 / 单次、独占 / 叠加：两种状态没有好坏之分，只换图标不变色。
  // Loop / once and exclusive / overlap: neither state is "better", so only the icon changes.
  // 循环 / 单次：像音乐播放器的循环按钮，点一下切换。Loop / once, like a music player.
  const loop = el('button', 'chip icon-chip toggle');
  loop.type = 'button';
  loop.innerHTML = s.looping ? icons.repeat : icons.repeatOnce;
  loop.title = s.looping ? t('循环播放（再按一下卡片停止）\n点击改为单次') : t('单次播放\n点击改为循环');
  loop.setAttribute('aria-pressed', String(s.looping));
  loop.addEventListener('click', (ev) => {
    ev.stopPropagation();
    patchSound(s.id, { looping: !s.looping }, true);
  });

  // 独占 / 叠加：与循环同样的图标按钮，点一下切换。Exclusive / overlap: same icon button style.
  const excl = el('button', 'chip icon-chip toggle');
  excl.type = 'button';
  excl.innerHTML = s.exclusive ? icons.exclusive : icons.overlap;
  excl.title = s.exclusive ? t('独占：播放时打断其它音效\n点击改为叠加') : t('叠加：可与其它音效同时播放\n点击改为独占');
  excl.setAttribute('aria-pressed', String(s.exclusive));
  excl.addEventListener('click', (ev) => {
    ev.stopPropagation();
    patchSound(s.id, { exclusive: !s.exclusive }, true);
  });

  const vol = el('button', `chip${s.gain_db !== 0 ? ' changed' : ''}`);
  vol.type = 'button';
  vol.innerHTML = `${icons.speaker}<span>${formatGain(s.gain_db)}</span>`;
  vol.title = t('音量：点击后上下拖动');
  vol.addEventListener('click', (ev) => {
    ev.stopPropagation();
    openGainSlider(s, vol);
  });

  // 倍速：纯文字，点一次切到下一档，右键回到上一档。
  // Speed: text only; each click steps to the next preset, right-click steps back.
  const speed = el('button', `chip${s.speed !== 1 ? ' changed' : ''}`, formatSpeed(s.speed));
  speed.type = 'button';
  speed.title = t('倍速（变速同时变调）：点击切换，右键反向');
  const step = (dir: 1 | -1) => {
    const next = nextSpeed(s.speed, dir);
    s.speed = next;
    speed.textContent = formatSpeed(next);
    speed.classList.toggle('changed', next !== 1);
    patchSound(s.id, { speed: next }, false);
  };
  speed.addEventListener('click', (ev) => {
    ev.stopPropagation();
    step(1);
  });
  speed.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    step(-1);
  });

  row.append(loop, excl, vol, speed);
  return row;
}

/** 倍速档位中的下一档 / 上一档（循环）；当前值不在档位里时取最近的一档。
 *  The next / previous speed preset (wrapping); off-preset values snap to the nearest. */
function nextSpeed(current: number, dir: 1 | -1): number {
  const n = SPEED_PRESETS.length;
  if (dir > 0) {
    const i = SPEED_PRESETS.findIndex((p) => p > current + 1e-6);
    return SPEED_PRESETS[i < 0 ? 0 : i];
  }
  const below = SPEED_PRESETS.filter((p) => p < current - 1e-6);
  return below.length ? below[below.length - 1] : SPEED_PRESETS[n - 1];
}

/** 进度条：播放中显示，按住拖动可跳转。Progress bar: shown while playing; drag to seek. */
function renderProgress(s: Sound): HTMLElement {
  const bar = el('div', 'progress');
  bar.innerHTML = '<div class="progress-track"><div class="progress-fill"></div></div><div class="progress-knob"></div>';
  bar.title = t('拖动跳转');
  const ratioAt = (clientX: number) => {
    const r = bar.getBoundingClientRect();
    return Math.max(0, Math.min(1, (clientX - r.left) / r.width));
  };
  bar.addEventListener('click', (ev) => ev.stopPropagation());
  bar.addEventListener('pointerdown', (ev) => {
    ev.stopPropagation();
    ev.preventDefault();
    bar.setPointerCapture(ev.pointerId);
    seeking.add(s.id);
    bar.classList.add('dragging');
    setProgress(bar, ratioAt(ev.clientX), s);
  });
  bar.addEventListener('pointermove', (ev) => {
    if (seeking.has(s.id)) setProgress(bar, ratioAt(ev.clientX), s);
  });
  const finish = (ev: PointerEvent) => {
    if (!seeking.has(s.id)) return;
    const ratio = ratioAt(ev.clientX);
    seeking.delete(s.id);
    bar.classList.remove('dragging');
    api.seek(s.id, ratio).catch((e) => toast(errorText(e), 'error'));
  };
  bar.addEventListener('pointerup', finish);
  bar.addEventListener('pointercancel', finish);
  return bar;
}

/** 画进度并更新时间文字。Draw progress and update the time text. */
function setProgress(bar: HTMLElement, ratio: number, s: Sound): void {
  (bar.querySelector('.progress-fill') as HTMLElement).style.width = `${ratio * 100}%`;
  (bar.querySelector('.progress-knob') as HTMLElement).style.left = `${ratio * 100}%`;
  const time = bar.parentElement?.querySelector('.pad-time');
  if (time && s.duration_secs != null) {
    time.textContent = `${formatDuration(ratio * s.duration_secs)} / ${formatDuration(s.duration_secs)}`;
  }
}

/** 只更新播放状态与进度，不重绘网格。Update playing state and progress only. */
function applyPlaying(): void {
  document.querySelectorAll<HTMLElement>('.pad').forEach((pad) => {
    const id = Number(pad.dataset.id);
    const progress = playing.get(id);
    const isPlaying = progress !== undefined;
    const was = pad.classList.contains('playing');
    pad.classList.toggle('playing', isPlaying);
    const bar = pad.querySelector<HTMLElement>('.progress');
    const s = library.sounds.find((x) => x.id === id);
    if (!bar || !s) return;
    if (isPlaying && !seeking.has(id)) setProgress(bar, progress, s);
    if (was && !isPlaying) {
      // 播完：时间文字恢复为总时长。Finished: restore the total duration text.
      const time = pad.querySelector('.pad-time');
      if (time) time.textContent = formatDuration(s.duration_secs);
    }
  });
}

/** 修改音效并更新本地数据；`rerender` 为真时重绘界面。
 *  Patch a sound and update local data; re-render when `rerender` is set. */
async function patchSound(id: number, patch: SoundPatch, rerender: boolean): Promise<Sound | null> {
  try {
    const updated = await api.updateSound(id, patch);
    const i = library.sounds.findIndex((s) => s.id === id);
    if (i >= 0) library.sounds[i] = updated;
    // 改名 / 移动会改变排序与文件夹统计，重新拉取整个库。
    // Renames / moves change ordering and folder counts; refetch the whole library.
    if (patch.name !== undefined || patch.folder !== undefined) await loadLibrary();
    else if (rerender) {
      renderGrid();
      renderSidebar();
    }
    return updated;
  } catch (e) {
    toast(t('保存失败：{error}', { error: errorText(e) }), 'error');
    return null;
  }
}

// ---- 弹出层 Popover ----

let popoverAnchor: HTMLElement | null = null;
let popoverOnClose: (() => void) | null = null;

/** 在锚点附近打开弹出层。`flyout`：紧凑样式，居中放在锚点上方（竖向滑条用）。
 *  Open the popover near an anchor. `flyout`: compact, centred above the anchor (for
 *  the vertical slider). */
function openPopover(anchor: HTMLElement, content: HTMLElement, opts: { flyout?: boolean; onClose?: () => void } = {}): void {
  closePopover();
  const pop = $('popover');
  pop.innerHTML = '';
  pop.appendChild(content);
  pop.classList.toggle('flyout', !!opts.flyout);
  pop.hidden = false;
  popoverAnchor = anchor;
  popoverOnClose = opts.onClose ?? null;
  anchor.classList.add('open');
  const a = anchor.getBoundingClientRect();
  const p = pop.getBoundingClientRect();
  if (opts.flyout) {
    // 优先放在锚点上方，空间不够时放下方；水平居中。Prefer above, else below; centred.
    const above = a.top - 8 - p.height > 8;
    pop.style.top = `${above ? a.top - 8 - p.height : a.bottom + 8}px`;
    pop.style.left = `${Math.max(8, Math.min(a.left + a.width / 2 - p.width / 2, window.innerWidth - p.width - 8))}px`;
  } else {
    // 优先放在锚点下方，空间不够时放上方；水平方向不超出窗口。
    // Prefer below the anchor, above if there's no room; keep within the window horizontally.
    const below = a.bottom + 8 + p.height < window.innerHeight;
    pop.style.top = `${below ? a.bottom + 8 : Math.max(8, a.top - 8 - p.height)}px`;
    pop.style.left = `${Math.max(8, Math.min(a.left, window.innerWidth - p.width - 8))}px`;
  }
  syncAllRangeFill(pop);
}

function closePopover(): void {
  const pop = $('popover');
  if (pop.hidden) return;
  pop.hidden = true;
  popoverAnchor?.classList.remove('open');
  popoverAnchor = null;
  const cb = popoverOnClose;
  popoverOnClose = null;
  cb?.();
}

function setupPopover(): void {
  document.addEventListener('pointerdown', (e) => {
    const pop = $('popover');
    if (pop.hidden) return;
    const t = e.target as Node;
    if (!pop.contains(t) && !popoverAnchor?.contains(t)) closePopover();
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') closePopover();
  });
  $('grid').addEventListener('scroll', closePopover);
  window.addEventListener('resize', closePopover);
  $('popover').addEventListener('click', (e) => e.stopPropagation());
}

/** 音量：在按钮上方弹出一根竖向滑条，拖动即生效（停手 250 ms 后保存），双击回到 0 dB。
 *  Volume: a vertical slider pops up above the chip; dragging applies immediately
 *  (saved 250 ms after it settles); double-click resets to 0 dB. */
function openGainSlider(s: Sound, chip: HTMLElement): void {
  const box = el('div', 'vslider');
  const value = el('span', 'value', formatGain(s.gain_db));
  const range = el('input', 'vertical');
  range.type = 'range';
  range.min = '-30';
  range.max = '12';
  range.step = '1';
  range.value = String(Math.round(s.gain_db));
  range.setAttribute('aria-label', t('{name} 音量', { name: s.name }));

  let timer = 0;
  const set = (v: number) => {
    value.textContent = formatGain(v);
    chip.querySelector('span')!.textContent = formatGain(v);
    chip.classList.toggle('changed', v !== 0);
    s.gain_db = v;
    clearTimeout(timer);
    timer = window.setTimeout(() => patchSound(s.id, { gain_db: v }, false), 250);
  };
  range.addEventListener('input', () => set(Number(range.value)));
  range.addEventListener('dblclick', () => {
    range.value = '0';
    syncRangeFill(range);
    set(0);
  });
  box.append(value, range);
  openPopover(chip, box, { flyout: true });
}

// ---- 通用输入 / 确认对话框 Generic prompt / confirm ----

/** 弹出输入框；`confirmOnly` 时只做确认（返回 '' 表示确认）。取消返回 null。
 *  Show a prompt; with `confirmOnly` it's a confirmation ('' = confirmed). Cancel → null. */
function promptText(opts: { title: string; message?: string; value?: string; okText?: string; confirmOnly?: boolean }): Promise<string | null> {
  const dialog = $<HTMLDialogElement>('prompt-dialog');
  const input = $<HTMLInputElement>('prompt-input');
  $('prompt-title').textContent = opts.title;
  $('prompt-message').textContent = opts.message ?? '';
  $('prompt-message').hidden = !opts.message;
  input.hidden = !!opts.confirmOnly;
  input.value = opts.value ?? '';
  const ok = $<HTMLButtonElement>('prompt-ok');
  ok.textContent = opts.okText ?? t('确定');
  ok.classList.toggle('danger', !!opts.confirmOnly);
  return new Promise((resolve) => {
    let result: string | null = null;
    const form = $<HTMLFormElement>('prompt-form');
    const onSubmit = (e: Event) => {
      e.preventDefault();
      result = opts.confirmOnly ? '' : input.value.trim();
      if (!opts.confirmOnly && !result) return;
      dialog.close();
    };
    form.addEventListener('submit', onSubmit);
    dialog.addEventListener(
      'close',
      () => {
        form.removeEventListener('submit', onSubmit);
        resolve(result);
      },
      { once: true },
    );
    dialog.showModal();
    if (!opts.confirmOnly) {
      input.focus();
      input.select();
    }
  });
}

// ---- 导入 Import ----

async function importFiles(files: FileList | File[], folder: string): Promise<void> {
  const list = Array.from(files);
  if (list.length === 0) return;
  let ok = 0;
  for (const f of list) {
    try {
      await api.importSound(f, folder);
      ok++;
    } catch (e) {
      toast(t('「{name}」导入失败：{error}', { name: f.name, error: errorText(e) }), 'error');
    }
  }
  if (ok) {
    const where = folder ? t('「{folder}」', { folder: folderName(folder) }) : t('音效库');
    toast(t('已导入 {n} 个音效到{where}', { n: ok, where }));
  }
  await loadLibrary();
}

function dropText(): string {
  return currentFolder ? t('松开即可导入到「{folder}」', { folder: folderName(currentFolder) }) : t('松开即可导入音效');
}

function hideDropOverlay(): void {
  $('drop-overlay').hidden = true;
  dragDepth = 0;
}

let dragDepth = 0;

function setupImport(): void {
  const input = $<HTMLInputElement>('file-input');
  $('import').addEventListener('click', () => input.click());
  input.addEventListener('change', () => {
    if (input.files) importFiles(input.files, currentFolder);
    input.value = '';
  });

  // 从系统拖入文件：显示遮罩；放在文件夹上导入到该文件夹，否则导入到当前文件夹。
  // Files dragged in from the OS: show the overlay; drop on a folder to import there,
  // anywhere else to import into the current folder.
  const overlay = $('drop-overlay');
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');
  window.addEventListener('dragenter', (e) => {
    if (!hasFiles(e)) return;
    dragDepth++;
    $('drop-target').textContent = dropText();
    overlay.hidden = false;
  });
  window.addEventListener('dragleave', (e) => {
    if (!hasFiles(e)) return;
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) overlay.hidden = true;
  });
  window.addEventListener('dragover', (e) => {
    if (hasFiles(e)) e.preventDefault();
  });
  window.addEventListener('drop', (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    hideDropOverlay();
    if (e.dataTransfer?.files) importFiles(e.dataTransfer.files, currentFolder);
  });
}

// ---- 编辑对话框 Edit dialog ----

let editing: Sound | null = null;
let editHotkey: string | null = null;

/** 编辑框里「不拦截」开关是否打开。Whether the editor's pass-through switch is on. */
function editPassthrough(): boolean {
  return $<HTMLInputElement>('edit-passthrough').checked;
}

/** 修饰键统一成后端的写法（左右不分）。Normalise modifiers to the backend's names (no left / right). */
function normalizeKey(code: string): string {
  if (/^Control(Left|Right)$/.test(code)) return 'Ctrl';
  if (/^Shift(Left|Right)$/.test(code)) return 'Shift';
  if (/^Alt(Left|Right)$/.test(code)) return 'Alt';
  if (/^(Meta|OS)(Left|Right)$/.test(code)) return 'Super';
  return code;
}

/** 系统热键（拦截）能否表示这个组合：前面只能是修饰键，且不含鼠标键。
 *  Whether a system (blocking) hotkey can express the combo: only modifiers before the
 *  last key, and no mouse buttons. */
function fitsSystemHotkey(hk: string): boolean {
  const parts = hk.split('+');
  const last = parts.pop() ?? '';
  return !/^Mouse/.test(last) && parts.every((p) => ['Ctrl', 'Alt', 'Shift', 'Super'].includes(p));
}

/** 单独一个键是否允许（字母 / 数字会挡住打字）。Whether a single key is allowed (letters /
 *  digits would get in the way of typing). */
function singleKeyAllowed(hk: string): boolean {
  return /^(F\d{1,2}|Numpad.*|Pause|ScrollLock|Insert|Home|End|PageUp|PageDown|Mouse.*)$/.test(hk);
}

/** 更新快捷键说明（两种方式录法不同）。Update the hotkey hint (the two kinds record differently). */
function renderHotkeyHint(): void {
  $('hotkey-hint').textContent = editPassthrough()
    ? t('点击后先按住对讲键等，再按触发键，然后松开；也可以用鼠标侧键。Esc 取消')
    : t('点击后按下组合键（其他程序全屏时也能触发），Esc 取消');
}
let deleteArmed = false;

function renderHotkeyButton(recording = false): void {
  const btn = $('hotkey-rec');
  btn.classList.toggle('recording', recording);
  btn.classList.toggle('has-key', !!editHotkey && !recording);
  btn.innerHTML = '';
  if (recording) btn.textContent = t('请按下组合键…');
  else if (editHotkey) btn.appendChild(kbd(editHotkey, editPassthrough()));
  else btn.textContent = t('点击设置快捷键');
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
  // 「不拦截」录制：按下顺序记录按住的键，第一次松开时定下组合（最后按下的是触发键）。
  // Pass-through recording: keys held, in press order; the combo is fixed on the first
  // release (the last key pressed is the trigger).
  let held: string[] = [];
  const mouseNames: Record<number, string> = { 1: 'MouseMiddle', 3: 'Mouse4', 4: 'Mouse5' };

  const stop = () => {
    recording = false;
    held = [];
    window.removeEventListener('keydown', onKey, true);
    window.removeEventListener('keyup', onKeyUp, true);
    window.removeEventListener('mousedown', onMouseDown, true);
    window.removeEventListener('mouseup', onMouseUp, true);
    window.removeEventListener('auxclick', swallow, true);
    renderHotkeyButton(false);
  };
  const accept = (hk: string) => {
    // 单独的字母 / 数字会挡住打字，要求组合（功能键、小键盘、鼠标侧键等除外）。
    // A lone letter / digit would get in the way of typing; require a combo (function
    // keys, numpad, mouse buttons and the like excepted).
    if (!hk.includes('+') && !singleKeyAllowed(hk)) {
      toast(t('单个字母或数字会影响打字，请加上 Ctrl / Alt / Shift'), 'error');
      held = [];
      return;
    }
    editHotkey = hk;
    stop();
  };
  const swallow = (e: Event) => {
    e.preventDefault();
    e.stopPropagation();
  };

  const onKey = (e: KeyboardEvent) => {
    swallow(e);
    if (e.code === 'Escape' && !held.length) return stop();
    if (editPassthrough()) {
      if (e.repeat) return;
      const key = normalizeKey(e.code);
      if (!held.includes(key)) held.push(key);
      return;
    }
    const hk = hotkeyFromEvent(e);
    if (hk) accept(hk);
  };
  const onKeyUp = (e: KeyboardEvent) => {
    swallow(e);
    if (editPassthrough() && held.length) accept(held.join('+'));
  };
  // 鼠标中键 / 侧键（只在「不拦截」方式下可用）；拦下浏览器的「后退 / 前进」。
  // Middle / side mouse buttons (pass-through only); block the browser's back / forward.
  const onMouseDown = (e: MouseEvent) => {
    const name = mouseNames[e.button];
    if (!name || !editPassthrough()) return;
    swallow(e);
    if (!held.includes(name)) held.push(name);
  };
  const onMouseUp = (e: MouseEvent) => {
    if (!mouseNames[e.button] || !editPassthrough()) return;
    swallow(e);
    if (held.length) accept(held.join('+'));
  };

  btn.addEventListener('click', () => {
    if (recording) return stop();
    recording = true;
    held = [];
    renderHotkeyButton(true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('keyup', onKeyUp, true);
    window.addEventListener('mousedown', onMouseDown, true);
    window.addEventListener('mouseup', onMouseUp, true);
    window.addEventListener('auxclick', swallow, true);
  });
  $('hotkey-clear').addEventListener('click', () => {
    editHotkey = null;
    stop();
  });
  // 切到「拦截」时，系统热键表示不了的组合（如对讲键 + 数字、鼠标键）要清掉重录。
  // Switching to blocking: combos a system hotkey can't express (push-to-talk + digit,
  // mouse buttons) must be cleared and recorded again.
  $('edit-passthrough').addEventListener('change', () => {
    stop();
    if (editHotkey && !editPassthrough() && !fitsSystemHotkey(editHotkey)) {
      editHotkey = null;
      toast(t('这个组合只能用「不拦截」方式，请重新录制'));
    }
    renderHotkeyHint();
    renderHotkeyButton(false);
  });
  $<HTMLDialogElement>('edit-dialog').addEventListener('close', stop);
}

function openEditor(s: Sound): void {
  closePopover();
  editing = s;
  deleteArmed = false;
  const form = $<HTMLFormElement>('edit-form');
  (form.elements.namedItem('name') as HTMLInputElement).value = s.name;
  const folderSel = $<HTMLSelectElement>('edit-folder');
  folderSel.innerHTML = '';
  folderSel.appendChild(new Option(t('（根目录）'), ''));
  for (const f of library.folders) folderSel.appendChild(new Option(f, f));
  folderSel.value = s.folder;
  (form.elements.namedItem('gain_db') as HTMLInputElement).value = String(s.gain_db);
  (form.elements.namedItem('speed') as HTMLInputElement).value = String(s.speed);
  updateSliderLabels();
  $<HTMLInputElement>('edit-looping').checked = s.looping;
  $<HTMLInputElement>('edit-exclusive').checked = s.exclusive;
  editHotkey = s.hotkey;
  $<HTMLInputElement>('edit-passthrough').checked = s.hotkey_passthrough;
  renderHotkeyHint();
  renderHotkeyButton(false);
  $('edit-file').textContent = s.error
    ? t('文件：{file}（{error}）', { file: s.file_name, error: s.error })
    : t('文件：{file}', { file: s.file_name });
  $('edit-delete').querySelector('span:last-child')!.textContent = t('删除');
  $<HTMLDialogElement>('edit-dialog').showModal();
  syncAllRangeFill(form);
}

function updateSliderLabels(): void {
  const form = $<HTMLFormElement>('edit-form');
  const gain = Number((form.elements.namedItem('gain_db') as HTMLInputElement).value);
  const speed = Number((form.elements.namedItem('speed') as HTMLInputElement).value);
  $('gain-out').textContent = formatGain(gain);
  $('speed-out').textContent = formatSpeed(speed);
}

function setupEditor(): void {
  const dialog = $<HTMLDialogElement>('edit-dialog');
  const form = $<HTMLFormElement>('edit-form');
  form.addEventListener('input', updateSliderLabels);
  const speed = form.elements.namedItem('speed') as HTMLInputElement;
  speed.addEventListener('dblclick', () => {
    speed.value = '1';
    syncRangeFill(speed);
    updateSliderLabels();
  });
  setupHotkeyRecorder();

  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    if (!editing) return;
    const data = new FormData(form);
    const patch: SoundPatch = {
      looping: $<HTMLInputElement>('edit-looping').checked,
      exclusive: $<HTMLInputElement>('edit-exclusive').checked,
      gain_db: Number(data.get('gain_db')),
      speed: Number(data.get('speed')),
      hotkey: editHotkey,
      hotkey_passthrough: editPassthrough(),
    };
    const name = String(data.get('name') ?? '').trim();
    const folder = String(data.get('folder') ?? '');
    if (name !== editing.name) patch.name = name;
    if (folder !== editing.folder) patch.folder = folder;
    if (await patchSound(editing.id, patch, true)) dialog.close();
  });

  // 删除需要点两次确认（文件移到回收站）。Delete needs a second click (file goes to the bin).
  $('edit-delete').addEventListener('click', async () => {
    if (!editing) return;
    const label = $('edit-delete').querySelector('span:last-child')!;
    if (!deleteArmed) {
      deleteArmed = true;
      label.textContent = t('再点一次：移到回收站');
      return;
    }
    try {
      await api.removeSound(editing.id);
      dialog.close();
      toast(t('已删除「{name}」（在回收站可恢复）', { name: editing.name }));
      await loadLibrary();
    } catch (e) {
      toast(t('删除失败：{error}', { error: errorText(e) }), 'error');
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
  toggle.title = m.muted ? t('已静音，点击取消') : t('点击静音');
  $('mic-name').textContent = m.connected ? (m.device ?? t('麦克风')) : t('麦克风未连接');
  const hint = $('mic-hint');
  hint.textContent = m.connected ? (m.muted ? t('已静音') : `${m.sample_rate ?? ''} Hz`) : (m.note ?? t('等待麦克风连接…'));
  hint.classList.toggle('warn', !m.connected);
  hint.title = hint.textContent ?? '';
  const gain = $<HTMLInputElement>('mic-gain');
  if (document.activeElement !== gain) {
    gain.value = String(m.gain_db);
    syncRangeFill(gain);
    $('mic-gain-value').textContent = formatGain(m.gain_db);
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
    $('mic-gain-value').textContent = formatGain(v);
    clearTimeout(timer);
    timer = window.setTimeout(() => api.setMic({ gain_db: v }).catch(() => {}), 120);
  });
  gain.addEventListener('dblclick', () => {
    gain.value = '0';
    gain.dispatchEvent(new Event('input'));
  });
}

// ---- 音频链路 Audio chain ----

/** 你的麦克风下拉框：硬件麦克风在前，虚拟设备单独一组。
 *  "Your mic" select: hardware mics first, virtual devices in their own group. */
function renderMicSelect(): void {
  const select = $<HTMLSelectElement>('mic-select');
  if (document.activeElement === select) return;
  const selection = mic?.selection ?? null;
  select.innerHTML = '';
  select.appendChild(new Option(t('自动（优先系统默认的硬件麦克风）'), ''));
  const hardware = inputs.filter((d) => !d.virtual);
  const virtual = inputs.filter((d) => d.virtual);
  const group = (label: string, items: InputItem[]) => {
    if (!items.length) return;
    const g = document.createElement('optgroup');
    g.label = label;
    for (const d of items) g.appendChild(new Option(d.is_default ? t('{name}（系统默认）', { name: d.name }) : d.name, d.name));
    select.appendChild(g);
  };
  group(t('麦克风'), hardware);
  group(t('虚拟设备（一般不用作麦克风）'), virtual);
  // 选中的设备暂时不在（如蓝牙已断开）：仍列出。Chosen device absent: still list it.
  if (selection && !inputs.some((d) => d.name === selection)) {
    select.appendChild(new Option(t('{name}（未连接）', { name: selection }), selection));
  }
  select.value = selection ?? '';
}

function renderMicStatusInSettings(): void {
  if (!mic) return;
  $('mic-status').textContent = mic.connected
    ? t('正在使用：{device}（{rate} Hz）', { device: mic.device ?? '', rate: mic.sample_rate ?? '' })
    : (mic.note ?? t('等待麦克风连接…'));
}

/** 输出音频线路下拉框：虚拟音频线（推荐）与扬声器分组。
 *  "Output line" select: virtual cables (recommended) and speakers grouped. */
function renderOutputSelect(current: OutputInfo): void {
  const select = $<HTMLSelectElement>('output-select');
  if (document.activeElement === select) return;
  select.innerHTML = '';
  const group = (label: string, items: OutputItem[]) => {
    if (!items.length) return;
    const g = document.createElement('optgroup');
    g.label = label;
    for (const d of items) g.appendChild(new Option(d.name, d.name));
    select.appendChild(g);
  };
  group(t('虚拟音频线（推荐，其他软件可当作麦克风）'), outputs.filter((o) => o.kind === 'virtual'));
  group(t('扬声器 / 耳机（只有自己能听到）'), outputs.filter((o) => o.kind === 'speaker'));
  if (!outputs.some((o) => o.name === current.output_device)) {
    select.appendChild(new Option(current.output_device, current.output_device));
  }
  select.value = current.output_device;
}

/** 底部「输出目标」与设置里的提示：其他软件里该选哪个麦克风，或扬声器警告。
 *  Bottom "send target" and the settings hint: which mic to pick, or a speaker warning. */
function renderSendTarget(o: OutputInfo): void {
  const btn = $('send-target');
  const warn = o.output_kind === 'speaker';
  btn.classList.toggle('warn', warn);
  btn.querySelector('.send-icon')!.innerHTML = warn ? icons.warning : icons.send;
  btn.title = warn
    ? t('混音正在输出到「{device}」，只有你自己能听到。点击在设置里改为虚拟音频线（如 CABLE Input）。', { device: o.output_device })
    : t('你的声音 + 音效输出到「{device}」；其他软件的麦克风选「{mic}」。', {
        device: o.output_device,
        mic: o.game_mic ?? o.output_device,
      });

  const hint = $('game-mic-hint');
  hint.classList.toggle('warn', warn);
  hint.innerHTML = '';
  if (warn) {
    appendTemplate(
      hint,
      t('当前输出是扬声器：只有你自己能听到，其他软件收不到。请选择一条虚拟音频线（如 {cable}），没有的话需要先安装 VB-Cable。'),
      { cable: el('code', '', 'CABLE Input') },
    );
  } else {
    appendTemplate(hint, t('其他软件的麦克风选 {mic}'), {
      mic: el('code', '', o.game_mic ?? o.output_device),
    });
  }
}

/** 试听音量滑条：设置里一个，底部抽屉里一个，两者保持同步。
 *  Monitor volume sliders: one in settings, one in the bottom drawer, kept in sync. */
const MONITOR_GAINS = ['monitor-gain', 'monitor-gain-bar'];

/** 试听：开关、设备、音量、状态，以及底部的耳机按钮。
 *  Monitor: switch, device, volume, status, and the headphones button at the bottom. */
function renderMonitor(m: MonitorStatus): void {
  monitor = m;
  $<HTMLInputElement>('monitor-toggle').checked = m.enabled;
  const btn = $('monitor-btn');
  btn.innerHTML = icons.headphones;
  btn.classList.toggle('on', m.enabled && m.active);
  btn.classList.toggle('idle', m.enabled && !m.active);
  btn.title = m.enabled
    ? m.active
      ? t('试听中：音效同时在「{device}」播放（点击关闭）', { device: m.device ?? '' })
      : t('试听已开启，但没有生效：{note}', { note: m.note ?? '' })
    : t('试听已关闭：点击开启，音效同时在耳机 / 扬声器播放');
  $('monitor-group').classList.toggle('enabled', m.enabled);

  const select = $<HTMLSelectElement>('monitor-select');
  if (document.activeElement !== select) {
    select.innerHTML = '';
    select.appendChild(new Option(t('系统默认播放设备'), ''));
    for (const o of outputs.filter((x) => x.kind === 'speaker')) select.appendChild(new Option(o.name, o.name));
    if (m.selection && !outputs.some((o) => o.name === m.selection)) {
      select.appendChild(new Option(t('{name}（未连接）', { name: m.selection }), m.selection));
    }
    select.value = m.selection ?? '';
    select.disabled = !m.enabled;
  }
  for (const id of MONITOR_GAINS) {
    const gain = $<HTMLInputElement>(id);
    if (document.activeElement === gain) continue;
    gain.value = String(m.gain_db);
    syncRangeFill(gain);
    $(`${id}-value`).textContent = formatGain(m.gain_db);
  }
  $('monitor-status').textContent = !m.enabled
    ? t('已关闭')
    : m.active
      ? t('正在试听：{device}', { device: m.device ?? '' })
      : (m.note ?? '');
}

function renderAppInfo(info: AppInfo): void {
  appInfo = info;
  applyTheme(info.theme_mode, info.accent);
  document.body.classList.toggle('is-app', info.is_app);
  document.body.classList.toggle('is-windows', info.platform === 'windows');
  renderSendTarget(info.output);
  renderRemote(info.remote);
  $('app-version').textContent = `v${info.version}`;
  $('data-dir').textContent = info.data_dir;
  const libInput = $<HTMLInputElement>('library-input');
  if (document.activeElement !== libInput) libInput.value = info.library_dir;
  $<HTMLInputElement>('tint-toggle').checked = info.tint_app_icon;
  renderDefaults(info.sound_defaults);
  if (view === 'sites') renderSites();
  const langSelect = $<HTMLSelectElement>('language-select');
  if (document.activeElement !== langSelect) langSelect.value = info.language;
  if (info.language_effective !== currentLang()) applyLanguage(info.language_effective);
  document.querySelectorAll<HTMLButtonElement>('[data-close-action]').forEach((b) =>
    b.classList.toggle('active', b.dataset.closeAction === info.close_action),
  );

  // Linux：虚拟麦克风开关与状态。Linux: virtual mic switch and status.
  $('vmic-box').hidden = info.platform !== 'linux';
  $<HTMLInputElement>('vmic-toggle').checked = info.auto_virtual_mic;
  const vm = info.virtual_mic;
  $('vmic-status').textContent = vm
    ? vm.active
      ? t('已创建：其他软件的麦克风选「{source}」', { source: vm.source })
      : t('未创建：{note}', { note: vm.note ?? '' })
    : t('未启用（已选择其它输出设备）');

  // 平台说明：标出当前平台。Platform help: mark the current platform.
  document.querySelectorAll<HTMLElement>('.platform').forEach((p) =>
    p.classList.toggle('current', p.dataset.platform === info.platform),
  );
}

async function loadDevices(): Promise<void> {
  try {
    const d = await api.devices();
    inputs = d.inputs;
    outputs = d.outputs;
    renderMicSelect();
    renderOutputSelect(d.current);
    renderSendTarget(d.current);
    renderMonitor(await api.monitor());
  } catch (e) {
    toast(t('无法读取设备列表：{error}', { error: errorText(e) }), 'error');
  }
}

async function openSettings(section: 'help' | 'audio' | null = null): Promise<void> {
  const dialog = $<HTMLDialogElement>('settings-dialog');
  if (!dialog.open) dialog.showModal();
  $(section === 'help' ? 'open-help' : 'open-settings').classList.add('active');
  syncAllRangeFill(dialog);
  loadDevices();
  try {
    renderAppInfo(await api.app());
  } catch (e) {
    toast(errorText(e), 'error');
  }
  if (section === 'help') {
    $<HTMLDetailsElement>('help-platforms').open = true;
    $('help-section').scrollIntoView({ behavior: 'smooth', block: 'start' });
  } else if (section === 'audio') {
    $('audio-section').scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
}

async function setMonitor(patch: { enabled?: boolean; device?: string | null; gain_db?: number }): Promise<void> {
  try {
    renderMonitor(await api.setMonitor(patch));
  } catch (e) {
    toast(t('试听设置失败：{error}', { error: errorText(e) }), 'error');
  }
}

function setupSettings(): void {
  const dialog = $<HTMLDialogElement>('settings-dialog');
  $('open-settings').addEventListener('click', () => openSettings());
  $('open-settings-mobile').addEventListener('click', () => openSettings());
  $('open-help').addEventListener('click', () => openSettings('help'));
  $('send-target').addEventListener('click', () => openSettings('audio'));
  dialog.addEventListener('close', () => {
    $('open-settings').classList.remove('active');
    $('open-help').classList.remove('active');
  });

  // 你的麦克风。Your mic.
  $('mic-refresh').addEventListener('click', loadDevices);
  const micSelect = $<HTMLSelectElement>('mic-select');
  micSelect.addEventListener('change', async () => {
    micSelect.disabled = true;
    try {
      renderMic(await api.selectMic(micSelect.value || null));
    } catch (e) {
      toast(t('切换麦克风失败：{error}', { error: errorText(e) }), 'error');
    } finally {
      micSelect.disabled = false;
      micSelect.blur();
      renderMicSelect();
    }
  });

  // 输出音频线路：切换后麦克风与试听会自动重开，设备列表也要刷新（反馈环过滤随之变化）。
  // Output line: the mic and monitor reopen automatically; refresh the device
  // lists too (the loop filter depends on the output).
  const outSelect = $<HTMLSelectElement>('output-select');
  outSelect.addEventListener('change', async () => {
    outSelect.disabled = true;
    try {
      const info = await api.selectOutput(outSelect.value || null);
      renderSendTarget(info);
      toast(
        info.output_kind === 'virtual'
          ? t('已输出到「{device}」', { device: info.output_device })
          : t('已切换到扬声器（其他软件收不到）'),
      );
    } catch (e) {
      toast(t('切换输出失败：{error}', { error: errorText(e) }), 'error');
    } finally {
      outSelect.disabled = false;
      outSelect.blur();
      await loadDevices();
    }
  });

  // 试听。Monitor.
  $('monitor-toggle').addEventListener('change', (ev) => setMonitor({ enabled: (ev.target as HTMLInputElement).checked }));
  $('monitor-btn').addEventListener('click', () => setMonitor({ enabled: !(monitor?.enabled ?? false) }));
  $<HTMLSelectElement>('monitor-select').addEventListener('change', (ev) => {
    const v = (ev.target as HTMLSelectElement).value;
    setMonitor({ device: v || null });
  });
  let mTimer = 0;
  for (const id of MONITOR_GAINS) {
    const mGain = $<HTMLInputElement>(id);
    mGain.addEventListener('input', () => {
      const v = Number(mGain.value);
      // 另一个滑条跟着动。Move the other slider along.
      for (const other of MONITOR_GAINS) {
        const o = $<HTMLInputElement>(other);
        if (o !== mGain) {
          o.value = mGain.value;
          syncRangeFill(o);
        }
        $(`${other}-value`).textContent = formatGain(v);
      }
      if (monitor) monitor.gain_db = v;
      clearTimeout(mTimer);
      mTimer = window.setTimeout(() => api.setMonitor({ gain_db: v }).catch(() => {}), 120);
    });
    mGain.addEventListener('dblclick', () => {
      mGain.value = '0';
      mGain.dispatchEvent(new Event('input'));
    });
  }

  $('vmic-toggle').addEventListener('change', async (ev) => {
    try {
      renderAppInfo(await api.setSettings({ auto_virtual_mic: (ev.target as HTMLInputElement).checked }));
      toast(t('重启 FerryMic 后生效'));
    } catch (e) {
      toast(errorText(e), 'error');
    }
  });

  // 窗口（仅 App）。Window (app only).
  $('tint-toggle').addEventListener('change', async (ev) => {
    try {
      renderAppInfo(await api.setSettings({ tint_app_icon: (ev.target as HTMLInputElement).checked }));
    } catch (e) {
      toast(errorText(e), 'error');
    }
  });
  document.querySelectorAll<HTMLButtonElement>('[data-close-action]').forEach((b) =>
    b.addEventListener('click', async () => {
      try {
        renderAppInfo(await api.setSettings({ close_action: b.dataset.closeAction as CloseAction }));
      } catch (e) {
        toast(errorText(e), 'error');
      }
    }),
  );

  // 音效库位置。Library location.
  const libInput = $<HTMLInputElement>('library-input');
  $('library-browse').addEventListener('click', async () => {
    try {
      const { path } = await api.pickFolder();
      if (path) libInput.value = path;
    } catch (e) {
      toast(t('无法打开文件夹选择框：{error}', { error: errorText(e) }), 'error');
    }
  });
  $('library-apply').addEventListener('click', async () => {
    const path = libInput.value.trim();
    if (!path || path === appInfo?.library_dir) return;
    const move = $<HTMLInputElement>('library-move').checked;
    try {
      applyLibrary(await api.setLibraryLocation(path, move));
      renderAppInfo(await api.app());
      toast(move ? t('已移动音效库到新位置') : t('已切换音效库位置'));
    } catch (e) {
      toast(t('修改位置失败：{error}', { error: errorText(e) }), 'error');
    }
  });
  const openLib = () => api.openLibrary().catch((e) => toast(errorText(e), 'error'));
  $('library-open').addEventListener('click', openLib);
  $('open-library').addEventListener('click', openLib);
}

// ---- 音效资源 Sound sites ----

let view: 'library' | 'sites' = 'library';
/** 音效资源页的搜索词。Search text on the sound sites page. */
let siteQuery = '';

/** 网站是否匹配搜索词（名称、网址、介绍、授权）。Whether a site matches the search text. */
function siteMatches(...fields: string[]): boolean {
  const q = siteQuery.trim().toLowerCase();
  return !q || fields.some((f) => f.toLowerCase().includes(q));
}

/** 切换主区域：音效库 / 音效资源。Switch the main area: library / sound sites. */
function setView(next: 'library' | 'sites'): void {
  view = next;
  document.body.classList.toggle('view-sites', next === 'sites');
  $('view-library').classList.toggle('active', next === 'library');
  $('view-sites').classList.toggle('active', next === 'sites');
  if (next === 'sites') renderSites();
}

function renderSites(): void {
  const grid = $('sites-grid');
  grid.innerHTML = '';
  const custom = appInfo?.custom_sites ?? [];

  const builtinHits = BUILTIN_SITES.filter((s) => siteMatches(s.name, s.url, t(s.desc), t(s.license), s.desc));
  const customHits = custom.map((c, i) => [c, i] as const).filter(([c]) => siteMatches(c.name, c.url));
  const searching = siteQuery.trim() !== '';

  if (builtinHits.length || !searching) {
    grid.append(el('h2', 'sites-heading', t('推荐网站')));
    const builtin = el('div', 'sites-cards');
    for (const s of builtinHits) builtin.append(siteCard(s.name, s.url, t(s.desc), t(s.license), null));
    grid.append(builtin);
  }

  // 搜索时没有命中就整组隐藏；不搜索时始终显示「我添加的」（含空状态提示）。
  // While searching, hide a group with no hits; otherwise always show "My sites".
  if (customHits.length || !searching) {
    grid.append(el('h2', 'sites-heading', t('我添加的')));
    const mine = el('div', 'sites-cards');
    if (!custom.length) mine.append(el('p', 'hint', t('还没有添加网址。点右上角「添加网址」，把常用的音效网站收藏到这里。')));
    for (const [c, i] of customHits) mine.append(siteCard(c.name, c.url, '', '', i));
    grid.append(mine);
  }

  if (searching && !builtinHits.length && !customHits.length) {
    grid.append(el('p', 'hint sites-empty', t('没有找到「{query}」相关的网站', { query: siteQuery.trim() })));
  }

  $('sites-count').textContent = t('{n} 个网站', { n: BUILTIN_SITES.length + custom.length });
}

function siteCard(name: string, url: string, desc: string, license: string, customIndex: number | null): HTMLElement {
  const card = el('div', 'site-card');
  const head = el('div', 'site-head');
  const titles = el('div', 'site-titles');
  titles.append(el('b', '', name), el('small', '', hostOf(url)));
  head.append(el('span', 'site-avatar', name.slice(0, 1).toUpperCase()), titles);
  card.append(head);
  if (desc) card.append(el('p', 'site-desc', desc));
  if (license) card.append(el('span', 'site-license', license));

  const actions = el('div', 'site-actions');
  const open = el('button', 'btn primary small');
  open.type = 'button';
  open.innerHTML = appInfo?.is_app ? `${icons.eye}<span>${t('预览')}</span>` : `${icons.globe}<span>${t('打开')}</span>`;
  open.title = url;
  open.addEventListener('click', () => openSite(url, name, false));
  actions.append(open);
  // App 里另给一个「在浏览器中打开」（需要登录的网站在常用浏览器里更方便）。
  // In the app, also offer "open in browser" (handy for sites needing your usual login).
  if (appInfo?.is_app) {
    const ext = iconButton('icon-btn small', 'globe', t('在浏览器中打开'));
    ext.addEventListener('click', () => openSite(url, name, true));
    actions.append(ext);
  }
  if (customIndex !== null) {
    const edit = iconButton('icon-btn small', 'edit', t('编辑'));
    edit.addEventListener('click', () => editSite(customIndex));
    const del = iconButton('icon-btn small', 'trash', t('删除'));
    del.addEventListener('click', () => deleteSite(customIndex));
    actions.append(el('div', 'spacer'), edit, del);
  }
  card.append(actions);
  return card;
}

/** 打开网站：App 里在预览窗口（或系统浏览器），浏览器版 / 手机上开新标签页。
 *  Open a site: the preview window (or system browser) in the app; a new tab in the
 *  browser version / on phones. */
async function openSite(url: string, name: string, external: boolean): Promise<void> {
  if (!appInfo?.is_app) {
    window.open(url, '_blank', 'noopener');
    return;
  }
  try {
    await api.openUrl(url, name, external);
  } catch (e) {
    toast(errorText(e), 'error');
  }
}

async function saveSites(list: CustomSite[]): Promise<void> {
  try {
    appInfo = await api.setSettings({ custom_sites: list });
    renderSites();
  } catch (e) {
    toast(t('保存失败：{error}', { error: errorText(e) }), 'error');
  }
}

/** 先问网址、再问名称（默认取主机名）。Ask for the address, then the name (host by default). */
async function askSite(initial?: CustomSite): Promise<CustomSite | null> {
  const raw = await promptText({
    title: initial ? t('编辑网址') : t('添加网址'),
    message: t('网址（如 freesound.org）'),
    value: initial?.url ?? '',
    okText: t('下一步'),
  });
  if (raw === null || !raw.trim()) return null;
  const url = normalizeUrl(raw);
  const name = await promptText({ title: t('网站名称'), message: url, value: initial?.name ?? hostOf(url) });
  if (name === null || !name.trim()) return null;
  return { name: name.trim(), url };
}

async function editSite(index: number): Promise<void> {
  const list = [...(appInfo?.custom_sites ?? [])];
  const next = await askSite(list[index]);
  if (!next) return;
  list[index] = next;
  await saveSites(list);
}

async function deleteSite(index: number): Promise<void> {
  const list = [...(appInfo?.custom_sites ?? [])];
  const ok = await promptText({
    title: t('删除网站'),
    message: t('删除「{name}」？', { name: list[index].name }),
    okText: t('删除'),
    confirmOnly: true,
  });
  if (ok === null) return;
  list.splice(index, 1);
  await saveSites(list);
}

function setupSites(): void {
  $('view-library').addEventListener('click', () => setView('library'));
  $('view-sites').addEventListener('click', () => setView('sites'));
  $<HTMLInputElement>('sites-search').addEventListener('input', (e) => {
    siteQuery = (e.target as HTMLInputElement).value;
    renderSites();
  });
  $('add-site').addEventListener('click', async () => {
    const site = await askSite();
    if (site) await saveSites([...(appInfo?.custom_sites ?? []), site]);
  });
}

// ---- 新音效的默认设置 Defaults for new sounds ----

function renderDefaults(d: SoundDefaults): void {
  $<HTMLInputElement>('def-looping').checked = d.looping;
  $<HTMLInputElement>('def-exclusive').checked = d.exclusive;
  const gain = $<HTMLInputElement>('def-gain');
  const speed = $<HTMLInputElement>('def-speed');
  if (document.activeElement !== gain) gain.value = String(d.gain_db);
  if (document.activeElement !== speed) speed.value = String(d.speed);
  $('def-gain-out').textContent = formatGain(Number(gain.value));
  $('def-speed-out').textContent = formatSpeed(Number(speed.value));
  syncRangeFill(gain);
  syncRangeFill(speed);
}

/** 读取设置里的默认值。Read the defaults from the settings form. */
function readDefaults(): SoundDefaults {
  return {
    looping: $<HTMLInputElement>('def-looping').checked,
    exclusive: $<HTMLInputElement>('def-exclusive').checked,
    gain_db: Number($<HTMLInputElement>('def-gain').value),
    speed: Number($<HTMLInputElement>('def-speed').value),
  };
}

function setupDefaults(): void {
  let timer = 0;
  const save = () => {
    clearTimeout(timer);
    timer = window.setTimeout(async () => {
      try {
        appInfo = await api.setSettings({ sound_defaults: readDefaults() });
      } catch (e) {
        toast(errorText(e), 'error');
      }
    }, 250);
  };
  for (const id of ['def-looping', 'def-exclusive']) $(id).addEventListener('change', save);
  const gain = $<HTMLInputElement>('def-gain');
  const speed = $<HTMLInputElement>('def-speed');
  gain.addEventListener('input', () => {
    $('def-gain-out').textContent = formatGain(Number(gain.value));
    save();
  });
  speed.addEventListener('input', () => {
    $('def-speed-out').textContent = formatSpeed(Number(speed.value));
    save();
  });
  // 双击恢复。Double-click to reset.
  gain.addEventListener('dblclick', () => {
    gain.value = '0';
    gain.dispatchEvent(new Event('input', { bubbles: true }));
  });
  speed.addEventListener('dblclick', () => {
    speed.value = '1';
    speed.dispatchEvent(new Event('input', { bubbles: true }));
  });

  // 应用到所有现有音效（快捷键、名称、文件夹不变）。
  // Apply to every existing sound (hotkeys, names and folders are unchanged).
  $('def-apply-all').addEventListener('click', async () => {
    await loadLibrary(); // 先同步磁盘上的变化。Pick up on-disk changes first.
    const n = library.sounds.length;
    if (!n) return;
    const ok = await promptText({
      title: t('应用到所有现有音效'),
      message: t('把这些默认设置应用到全部 {n} 个音效？快捷键不受影响。', { n }),
      okText: t('应用'),
      confirmOnly: true,
    });
    if (ok === null) return;
    const d = readDefaults();
    try {
      for (const s of library.sounds) {
        await api.updateSound(s.id, { looping: d.looping, exclusive: d.exclusive, gain_db: d.gain_db, speed: d.speed });
      }
      await loadLibrary();
      toast(t('已应用到 {n} 个音效', { n }));
    } catch (e) {
      toast(t('应用失败：{error}', { error: errorText(e) }), 'error');
      await loadLibrary();
    }
  });
}

// ---- 语言 Language ----

/** 切换语言：静态文字由 i18n 替换，动态内容重新渲染。
 *  Switch language: i18n swaps static text; dynamic content is re-rendered. */
function applyLanguage(lang: Lang): void {
  setLang(lang);
  renderSidebar();
  renderChips();
  renderGrid();
  renderHotkeyButton();
  if (mic) renderMic(mic);
  if (monitor) renderMonitor(monitor);
  if (appInfo) {
    renderSendTarget(appInfo.output);
    renderRemote(appInfo.remote);
  }
  renderMicSelect();
  if (view === 'sites') renderSites();
  if ($<HTMLDialogElement>('settings-dialog').open) loadDevices();
}

function setupLanguage(): void {
  const select = $<HTMLSelectElement>('language-select');
  select.addEventListener('change', async () => {
    try {
      // 后端也要切换（错误提示、托盘菜单）。The backend switches too (errors, tray menu).
      renderAppInfo(await api.setSettings({ language: select.value as LangPref }));
    } catch (e) {
      toast(errorText(e), 'error');
    }
  });
}

// ---- 手机遥控 Phone remote ----

function renderRemote(r: RemoteStatus): void {
  const btn = $('remote-btn');
  btn.classList.toggle('on', r.enabled);
  btn.title = r.enabled ? t('手机遥控：已开启（点击查看二维码）') : t('手机遥控：已关闭（点击开启）');
  $<HTMLInputElement>('remote-toggle').checked = r.enabled;
  $('remote-on').hidden = !r.enabled;
  $('remote-off').hidden = r.enabled;
  $('remote-url').textContent = r.url ?? '';
  $('remote-error').textContent = r.error ?? '';
  const qr = $<HTMLImageElement>('remote-qr');
  if (r.enabled && r.url && qr.dataset.url !== r.url) {
    qr.dataset.url = r.url;
    qr.src = `/api/remote/qr.svg?u=${encodeURIComponent(r.url)}`;
  }
}

async function setRemote(patch: { enabled?: boolean; regenerate?: boolean }): Promise<void> {
  try {
    renderRemote(await api.setRemote(patch));
  } catch (e) {
    toast(t('手机遥控：{error}', { error: errorText(e) }), 'error');
  }
}

function setupRemote(): void {
  const dialog = $<HTMLDialogElement>('remote-dialog');
  // 左侧按钮：未开启时直接开启并显示二维码；已开启时显示二维码。
  // Rail button: turns it on and shows the QR when off; shows the QR when on.
  $('remote-btn').addEventListener('click', async () => {
    dialog.showModal();
    try {
      const r = await api.remote();
      if (!r.enabled) await setRemote({ enabled: true });
      else renderRemote(r);
    } catch (e) {
      toast(errorText(e), 'error');
    }
  });
  $('remote-toggle').addEventListener('change', (ev) => setRemote({ enabled: (ev.target as HTMLInputElement).checked }));
  $('remote-regen').addEventListener('click', () => setRemote({ regenerate: true }));
  api
    .remote()
    .then(renderRemote)
    .catch(() => {});
}

// ---- 关闭窗口询问（桌面 App）Close prompt (desktop app) ----

function setupClosePrompt(): void {
  const dialog = $<HTMLDialogElement>('close-dialog');
  let answered = false;
  const answer = async (action: 'tray' | 'quit' | 'cancel') => {
    answered = true;
    const remember = action !== 'cancel' && $<HTMLInputElement>('close-remember').checked;
    dialog.close();
    try {
      await api.window(action, remember);
    } catch {
      /* 退出时连接会断开。The connection drops when quitting. */
    }
  };
  $('close-tray').addEventListener('click', () => answer('tray'));
  $('close-quit').addEventListener('click', () => answer('quit'));
  $('close-cancel').addEventListener('click', () => answer('cancel'));
  // 点遮罩 / Esc 关掉也算取消，否则下次点 × 会直接退出。
  // Closing via backdrop / Esc counts as cancel, or the next × click would quit right away.
  dialog.addEventListener('close', () => {
    if (!answered) api.window('cancel', false).catch(() => {});
  });
  dialog.addEventListener('open-prompt', () => {
    answered = false;
  });
}

function showClosePrompt(): void {
  const dialog = $<HTMLDialogElement>('close-dialog');
  if (dialog.open) return;
  $<HTMLInputElement>('close-remember').checked = false;
  dialog.dispatchEvent(new Event('open-prompt'));
  dialog.showModal();
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

// ---- 滑块填充 Slider fill ----

/** 按滑块当前值更新已填充比例（CSS 变量 --fill）。Update a slider's filled ratio (CSS var --fill). */
function syncRangeFill(input: HTMLInputElement): void {
  const min = Number(input.min || 0);
  const max = Number(input.max || 100);
  const pct = ((Number(input.value) - min) / (max - min)) * 100;
  input.style.setProperty('--fill', `${Math.max(0, Math.min(100, pct))}%`);
}

function syncAllRangeFill(root: ParentNode = document): void {
  root.querySelectorAll<HTMLInputElement>('input[type="range"]').forEach(syncRangeFill);
}

function setupRangeFill(): void {
  // 委托监听：动态创建的滑块（弹出层）也生效。Delegated: works for dynamic sliders too.
  document.addEventListener('input', (e) => {
    const t = e.target as HTMLInputElement;
    if (t.type === 'range') syncRangeFill(t);
  });
  syncAllRangeFill();
}

// ---- 轮询与同步 Polling and sync ----

function startPolling(): void {
  const live = async () => {
    try {
      const s = await api.live();
      playing = new Map(s.playing.map((p) => [p.id, p.progress]));
      applyPlaying();
      $('mic-meter').style.width = `${levelPercent(s.mic_level)}%`;
      if (s.close_prompt) showClosePrompt();
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

  // 未加载的音效加载完后时长才出现：有音效未加载时定期刷新一次。
  // Durations of not-yet-loaded sounds appear once loaded: refresh while any are pending.
  setInterval(() => {
    if (library.sounds.some((s) => !s.loaded && !s.error) && $('popover').hidden) loadLibrary();
  }, 3000);
}

async function rescan(silent: boolean): Promise<void> {
  try {
    applyLibrary(await api.rescan());
    if (!silent) toast(t('已重新扫描音效库'));
  } catch (e) {
    if (!silent) toast(t('扫描失败：{error}', { error: errorText(e) }), 'error');
  }
}

// ---- 启动 Start ----

hydrateIcons();
setupRangeFill();
setupDialogs();
setupPopover();
setupTheme();
setupImport();
setupEditor();
setupMicBar();
setupSettings();
setupRemote();
setupClosePrompt();
setupLanguage();
setupDefaults();
setupSites();
// 所有静态元素（含动态生成的色板）就绪后再翻译。Translate once every static element exists.
initLang();
$('new-folder').addEventListener('click', () => newFolder(currentFolder));
$('rescan').addEventListener('click', () => rescan(false));
$<HTMLInputElement>('search').addEventListener('input', (e) => {
  query = (e.target as HTMLInputElement).value;
  renderGrid();
});
$('stop-all').addEventListener('click', () => api.stopAll().catch((e) => toast(errorText(e), 'error')));
// 回到窗口时重新扫描（可能在资源管理器里改过文件）。Rescan on focus (files may have changed).
// 预览窗口里下载完成后，App 通知这里重新扫描。The app asks for a rescan after a preview download.
window.addEventListener('ferrymic:rescan', () => rescan(true));
let lastFocusScan = 0;
window.addEventListener('focus', () => {
  if (Date.now() - lastFocusScan > 2000) {
    lastFocusScan = Date.now();
    rescan(true);
  }
});
api
  .app()
  .then(renderAppInfo)
  .catch(() => {});
// 底部耳机按钮需要试听状态。The headphones button needs the monitor state.
api
  .monitor()
  .then(renderMonitor)
  .catch(() => {});
loadLibrary();
startPolling();
