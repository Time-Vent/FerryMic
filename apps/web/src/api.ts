// 后端 API 的类型与请求封装。
// Types and request helpers for the backend API.

export type ThemeMode = 'system' | 'dark' | 'light';
export type CloseAction = 'ask' | 'tray' | 'quit';
/** 输出类型：虚拟音频线（游戏收得到）/ 扬声器（只能自己听）。Output kind. */
export type OutputKind = 'virtual' | 'speaker';

export interface Sound {
  id: number;
  /** 名称 = 文件名去掉扩展名。Name = file name without extension. */
  name: string;
  /** 所在文件夹（相对音效库根目录，根目录为空串）。Folder (relative; "" = root). */
  folder: string;
  file_name: string;
  /** 循环播放（否 = 单次）。Loop (false = once). */
  looping: boolean;
  /** 独占（否 = 叠加）。Exclusive (false = overlap). */
  exclusive: boolean;
  gain_db: number;
  speed: number;
  hotkey: string | null;
  /** 时长（秒），加载后才知道。Duration (seconds), known once loaded. */
  duration_secs: number | null;
  /** 是否已解码进内存。Whether decoded into memory. */
  loaded: boolean;
  /** 加载失败原因。Load failure reason. */
  error: string | null;
}

export interface Library {
  root: string;
  /** 所有文件夹（相对路径，`/` 分隔，已排序）。All folders (relative, `/`-separated, sorted). */
  folders: string[];
  sounds: Sound[];
}

export interface MicStatus {
  /** 用户选择；null = 自动（优先系统默认的硬件麦克风）。User choice; null = auto. */
  selection: string | null;
  device: string | null;
  sample_rate: number | null;
  connected: boolean;
  note: string | null;
  gain_db: number;
  muted: boolean;
  level: number;
}

export interface LiveState {
  playing: { id: number; progress: number }[];
  mic_level: number;
  mic_connected: boolean;
  /** 桌面 App 请求弹出「关闭窗口」询问。The desktop app wants the close prompt. */
  close_prompt: boolean;
}

export interface InputItem {
  name: string;
  is_default: boolean;
  /** 虚拟设备（一般不当麦克风用）。Virtual device (normally not a mic). */
  virtual: boolean;
}

export interface OutputItem {
  name: string;
  is_default: boolean;
  kind: OutputKind;
  /** 选它时游戏里该选的麦克风。The mic games should pick for this output. */
  game_mic: string | null;
}

export interface OutputInfo {
  output_device: string;
  output_rate: number;
  output_channels: number;
  output_kind: OutputKind;
  game_mic: string | null;
}

export interface Devices {
  outputs: OutputItem[];
  inputs: InputItem[];
  current: OutputInfo;
}

export interface MonitorStatus {
  enabled: boolean;
  /** 选择的试听设备；null = 系统默认播放设备。Chosen device; null = default playback. */
  selection: string | null;
  device: string | null;
  active: boolean;
  note: string | null;
  gain_db: number;
}

export interface RemoteStatus {
  enabled: boolean;
  /** 手机访问网址（含访问码）。Phone URL (with access code). */
  url: string | null;
  port: number | null;
  error: string | null;
}

export interface VirtualMicInfo {
  active: boolean;
  source: string;
  note: string | null;
}

export interface AppInfo {
  version: string;
  platform: string;
  data_dir: string;
  library_dir: string;
  output: OutputInfo;
  port: number;
  remote: RemoteStatus;
  theme_mode: ThemeMode;
  accent: string;
  auto_virtual_mic: boolean;
  /** 是否在桌面 App 里运行。Running inside the desktop app. */
  is_app: boolean;
  tint_app_icon: boolean;
  close_action: CloseAction;
  /** 语言偏好。Language preference. */
  language: 'system' | 'zh-CN' | 'en';
  /** 实际使用的语言。The language actually in use. */
  language_effective: 'zh-CN' | 'en';
  virtual_mic: VirtualMicInfo | null;
}

/** 可编辑字段；hotkey 为 null 表示清除。Editable fields; hotkey null clears it. */
export interface SoundPatch {
  name?: string;
  folder?: string;
  looping?: boolean;
  exclusive?: boolean;
  gain_db?: number;
  speed?: number;
  hotkey?: string | null;
}

export interface SettingsPatch {
  theme_mode?: ThemeMode;
  accent?: string;
  auto_virtual_mic?: boolean;
  tint_app_icon?: boolean;
  close_action?: CloseAction;
  language?: 'system' | 'zh-CN' | 'en';
}

/** 发请求；非 2xx 时抛出带后端消息的错误。Send a request; throws with the backend message on non-2xx. */
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new Error(body || `${res.status} ${res.statusText}`);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

function json<T>(path: string, method: string, body: unknown): Promise<T> {
  return request<T>(path, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

export const api = {
  app: () => request<AppInfo>('/api/app'),
  setSettings: (s: SettingsPatch) => json<AppInfo>('/api/settings', 'POST', s),
  pickFolder: () => request<{ path: string | null }>('/api/pick-folder', { method: 'POST' }),
  live: () => request<LiveState>('/api/live'),
  window: (action: 'tray' | 'quit' | 'cancel', remember: boolean) =>
    json<void>('/api/window', 'POST', { action, remember }),

  devices: () => request<Devices>('/api/devices'),
  selectOutput: (device: string | null) => json<OutputInfo>('/api/output', 'POST', { device }),
  mic: () => request<MicStatus>('/api/mic'),
  setMic: (m: { gain_db?: number; muted?: boolean }) => json<MicStatus>('/api/mic', 'POST', m),
  selectMic: (device: string | null) => json<MicStatus>('/api/mic/device', 'POST', { device }),
  monitor: () => request<MonitorStatus>('/api/monitor'),
  setMonitor: (m: { enabled?: boolean; device?: string | null; gain_db?: number }) =>
    json<MonitorStatus>('/api/monitor', 'POST', m),

  remote: () => request<RemoteStatus>('/api/remote'),
  setRemote: (r: { enabled?: boolean; regenerate?: boolean }) => json<RemoteStatus>('/api/remote', 'POST', r),

  library: () => request<Library>('/api/library'),
  rescan: () => request<Library>('/api/library/rescan', { method: 'POST' }),
  setLibraryLocation: (path: string, move_contents: boolean) =>
    json<Library>('/api/library/location', 'POST', { path, move_contents }),
  openLibrary: () => request<void>('/api/library/open', { method: 'POST' }),
  createFolder: (path: string) => json<Library>('/api/folders', 'POST', { path }),
  renameFolder: (from: string, to: string) => json<Library>('/api/folders', 'PATCH', { from, to }),
  deleteFolder: (path: string) => json<Library>('/api/folders', 'DELETE', { path }),

  importSound: (file: File, folder: string) => {
    const form = new FormData();
    form.append('folder', folder);
    form.append('file', file, file.name);
    return request<Sound>('/api/sounds/import', { method: 'POST', body: form });
  },
  updateSound: (id: number, patch: SoundPatch) => json<Sound>(`/api/sounds/${id}`, 'PATCH', patch),
  removeSound: (id: number) => request<void>(`/api/sounds/${id}`, { method: 'DELETE' }),
  trigger: (id: number) => request<void>(`/api/sounds/${id}/trigger`, { method: 'POST' }),
  stop: (id: number) => request<void>(`/api/sounds/${id}/stop`, { method: 'POST' }),
  seek: (id: number, position: number) => json<void>(`/api/sounds/${id}/seek`, 'POST', { position }),
  stopAll: () => request<void>('/api/stop-all', { method: 'POST' }),
};
