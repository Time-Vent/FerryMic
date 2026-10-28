// 内联 SVG 图标（自绘，无外部依赖，跟随 currentColor）。
// Inline SVG icons (hand-drawn, no external dependency, follow currentColor).

/** 包一层 24×24 的描边 SVG。Wrap paths in a 24×24 stroked SVG. */
function svg(body: string, extra = ''): string {
  return `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" ${extra}>${body}</svg>`;
}

/** 生成齿轮轮廓（8 齿）。Build a gear outline (8 teeth). */
function gearPath(): string {
  const teeth = 8;
  const rOuter = 9.5;
  const rInner = 7.3;
  const pts: string[] = [];
  for (let i = 0; i < teeth; i++) {
    const base = (i / teeth) * Math.PI * 2;
    const step = (Math.PI * 2) / teeth;
    // 每个齿：内-外-外-内 四个点。Each tooth: inner-outer-outer-inner.
    const angles: [number, number][] = [
      [base - step * 0.32, rInner],
      [base - step * 0.18, rOuter],
      [base + step * 0.18, rOuter],
      [base + step * 0.32, rInner],
    ];
    for (const [a, r] of angles) {
      pts.push(`${(12 + r * Math.cos(a)).toFixed(2)} ${(12 + r * Math.sin(a)).toFixed(2)}`);
    }
  }
  return `M${pts.join(' L')} Z`;
}

export const icons = {
  logo: `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="9" width="2.6" height="6" rx="1.3" fill="currentColor"/><rect x="7.4" y="5" width="2.6" height="14" rx="1.3" fill="currentColor"/><rect x="11.8" y="7.5" width="2.6" height="9" rx="1.3" fill="currentColor"/><rect x="16.2" y="3.5" width="2.6" height="17" rx="1.3" fill="currentColor"/><rect x="20.4" y="10" width="2.2" height="4" rx="1.1" fill="currentColor"/></svg>`,
  grid: svg('<rect x="4" y="4" width="7" height="7" rx="2"/><rect x="13" y="4" width="7" height="7" rx="2"/><rect x="4" y="13" width="7" height="7" rx="2"/><rect x="13" y="13" width="7" height="7" rx="2"/>'),
  settings: svg(`<path d="${gearPath()}"/><circle cx="12" cy="12" r="3"/>`),
  play: `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.6-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z" fill="currentColor"/></svg>`,
  stop: `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="6.5" y="6.5" width="11" height="11" rx="2.2" fill="currentColor"/></svg>`,
  plus: svg('<path d="M12 5v14M5 12h14"/>'),
  search: svg('<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>'),
  more: `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="6" cy="12" r="1.7" fill="currentColor"/><circle cx="12" cy="12" r="1.7" fill="currentColor"/><circle cx="18" cy="12" r="1.7" fill="currentColor"/></svg>`,
  mic: svg('<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>'),
  micOff: svg('<path d="M15 9.3V6a3 3 0 0 0-5.7-1.3M9 9v2a3 3 0 0 0 4.6 2.5M5.5 11a6.5 6.5 0 0 0 10.4 5.2M18.5 11c0 .9-.2 1.8-.5 2.6M12 17.5V21M4 4l16 16"/>'),
  speaker: svg('<path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5H4z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>'),
  refresh: svg('<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4h-4"/>'),
  close: svg('<path d="M6 6l12 12M18 6 6 18"/>'),
  trash: svg('<path d="M4.5 7h15M9.5 7V5h5v2M6.5 7l1 12h9l1-12"/>'),
  upload: svg('<path d="M12 15V4M7.5 8.5 12 4l4.5 4.5M5 15v3.5A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5V15"/>'),
  phone: svg('<rect x="7" y="2.5" width="10" height="19" rx="2.5"/><path d="M11 18.5h2"/>'),
  info: svg('<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>'),
  stopAll: svg('<rect x="5" y="5" width="14" height="14" rx="3"/>'),
};
