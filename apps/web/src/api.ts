// 后端 API 的类型与请求封装。
// Types and request helpers for the backend API.

export type TriggerMode = 'OneShot' | 'Loop' | 'Exclusive' | 'Overlap';

export interface Sound {
  id: number;
  name: string;
  /** 音频文件名。Audio file name. */
  file_name: string;
  trigger_mode: TriggerMode;
  gain_db: number;
  speed: number;
  hotkey: string | null;
  /** 时长（秒）。Duration (seconds). */
  duration_secs: number | null;
  /** 加载失败原因。Load failure reason. */
  error: string | null;
}

export interface MicStatus {
  /** 用户选择；null = 跟随系统默认。User choice; null = follow system default. */
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
  playing: number[];
  mic_level: number;
  mic_connected: boolean;
}

export interface DeviceItem {
  name: string;
  is_default: boolean;
}

export interface OutputInfo {
  output_device: string;
  output_rate: number;
  output_channels: number;
}

export interface Devices {
  outputs: DeviceItem[];
  inputs: DeviceItem[];
  current: OutputInfo;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  output: OutputInfo;
  port: number;
  lan_remote: boolean;
  lan_active: boolean;
  lan_url: string | null;
}

/** 可编辑字段；hotkey 为 null 表示清除。Editable fields; hotkey null clears it. */
export interface SoundPatch {
  name?: string;
  trigger_mode?: TriggerMode;
  gain_db?: number;
  speed?: number;
  hotkey?: string | null;
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
  setSettings: (s: { lan_remote?: boolean }) => json<AppInfo>('/api/settings', 'POST', s),
  live: () => request<LiveState>('/api/live'),
  devices: () => request<Devices>('/api/devices'),
  mic: () => request<MicStatus>('/api/mic'),
  setMic: (m: { gain_db?: number; muted?: boolean }) => json<MicStatus>('/api/mic', 'POST', m),
  selectMic: (device: string | null) => json<MicStatus>('/api/mic/device', 'POST', { device }),
  sounds: () => request<Sound[]>('/api/sounds'),
  importSound: (file: File) => {
    const form = new FormData();
    form.append('file', file, file.name);
    return request<Sound>('/api/sounds/import', { method: 'POST', body: form });
  },
  updateSound: (id: number, patch: SoundPatch) => json<Sound>(`/api/sounds/${id}`, 'PATCH', patch),
  removeSound: (id: number) => request<void>(`/api/sounds/${id}`, { method: 'DELETE' }),
  trigger: (id: number) => request<void>(`/api/sounds/${id}/trigger`, { method: 'POST' }),
  stop: (id: number) => request<void>(`/api/sounds/${id}/stop`, { method: 'POST' }),
  stopAll: () => request<void>('/api/stop-all', { method: 'POST' }),
};
