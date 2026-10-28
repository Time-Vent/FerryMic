// SoundBridge 前端逻辑。
// SoundBridge front-end logic.

interface Sound {
  id: number;
  name: string;
  trigger_mode: 'OneShot' | 'Loop' | 'Exclusive' | 'Overlap';
  gain_db: number;
  speed: number;
  hotkey: string | null;
}

const modeLabel: Record<Sound['trigger_mode'], string> = {
  OneShot: '点按',
  Loop: '循环',
  Exclusive: '独占',
  Overlap: '叠加',
};

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    headers: { 'Content-Type': 'application/json' },
    ...init,
  });
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new Error(`${res.status}: ${body || res.statusText}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

async function loadSounds(): Promise<void> {
  const list = document.getElementById('sound-list')!;
  list.innerHTML = '';
  try {
    const sounds = await api<Sound[]>('/api/sounds');
    if (sounds.length === 0) {
      list.appendChild(el('p', 'empty', '还没有音效，先在上方添加一个。'));
      return;
    }
    for (const s of sounds) {
      list.appendChild(renderSound(s));
    }
    setStatus('已连接');
  } catch (e) {
    setStatus('连接失败');
    list.appendChild(el('p', 'empty', `无法连接后端：${(e as Error).message}`));
  }
}

function renderSound(s: Sound): HTMLElement {
  const card = el('div', 'sound');
  card.dataset.id = String(s.id);

  const head = el('div', 'sound-head');
  head.appendChild(el('span', 'sound-name', s.name));
  head.appendChild(el('span', 'sound-mode', modeLabel[s.trigger_mode]));
  card.appendChild(head);

  const meta = el('div', 'sound-meta');
  meta.appendChild(el('span', '', `音量 ${s.gain_db} dB`));
  meta.appendChild(el('span', '', `倍速 ${s.speed}x`));
  if (s.hotkey) meta.appendChild(el('span', 'hotkey', `键 ${s.hotkey}`));
  card.appendChild(meta);

  const btns = el('div', 'sound-btns');
  const trig = el('button', 'trigger', '▶ 播放');
  trig.addEventListener('click', () => trigger(s.id, trig));
  btns.appendChild(trig);

  const stop = el('button', 'stop', '■ 停止');
  stop.addEventListener('click', () => stopSound(s.id));
  btns.appendChild(stop);

  const del = el('button', 'del', '删除');
  del.addEventListener('click', () => removeSound(s.id));
  btns.appendChild(del);

  card.appendChild(btns);
  return card;
}

async function trigger(id: number, btn: HTMLButtonElement): Promise<void> {
  try {
    await api(`/api/sounds/${id}/trigger`, { method: 'POST' });
    btn.classList.add('active');
    setTimeout(() => btn.classList.remove('active'), 200);
  } catch (e) {
    alert(`触发失败：${(e as Error).message}`);
  }
}

async function stopSound(id: number): Promise<void> {
  try {
    await api(`/api/sounds/${id}/stop`, { method: 'POST' });
  } catch (e) {
    alert(`停止失败：${(e as Error).message}`);
  }
}

async function removeSound(id: number): Promise<void> {
  try {
    await api(`/api/sounds/${id}`, { method: 'DELETE' });
    loadSounds();
  } catch (e) {
    alert(`删除失败：${(e as Error).message}`);
  }
}

function setStatus(text: string): void {
  document.getElementById('conn-status')!.textContent = text;
}

function setupForm(): void {
  const form = document.getElementById('add-form') as HTMLFormElement;
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const body = {
      name: data.get('name'),
      file_path: data.get('file_path'),
      trigger_mode: data.get('trigger_mode'),
      gain_db: Number(data.get('gain_db') ?? 0),
      speed: Number(data.get('speed') ?? 1),
      hotkey: (data.get('hotkey') as string)?.trim() || null,
    };
    try {
      await api('/api/sounds', {
        method: 'POST',
        body: JSON.stringify(body),
      });
      form.reset();
      loadSounds();
    } catch (e) {
      alert(`添加失败：${(e as Error).message}`);
    }
  });
}

setupForm();
loadSounds();
