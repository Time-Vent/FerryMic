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
  folder: svg('<path d="M3.5 7.5A2 2 0 0 1 5.5 5.5h3.6l2 2.2h7.4a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>'),
  folderPlus: svg('<path d="M3.5 7.5A2 2 0 0 1 5.5 5.5h3.6l2 2.2h7.4a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/><path d="M12 11v5M9.5 13.5h5"/>'),
  library: svg('<path d="M4.5 19.5V5M8.5 19.5V5M12.5 19.5 16 5.2l3.8 1-3.5 14.3"/>'),
  chevron: svg('<path d="m9.5 6.5 5.5 5.5-5.5 5.5"/>'),
  speed: svg('<path d="M4.5 16.5a7.5 7.5 0 1 1 15 0"/><path d="m12 16.5 3.5-4.5"/>'),
  palette: svg('<path d="M12 3.5a8.5 8.5 0 1 0 0 17c1.2 0 1.6-.9 1.2-1.8-.5-1.1.2-2.2 1.4-2.2h2.1a3.3 3.3 0 0 0 3.3-3.3C20 7.2 16.4 3.5 12 3.5z"/><circle cx="7.8" cy="11" r="1.1" fill="currentColor"/><circle cx="10.5" cy="7.6" r="1.1" fill="currentColor"/><circle cx="14.8" cy="7.9" r="1.1" fill="currentColor"/>'),
  help: svg('<circle cx="12" cy="12" r="8.5"/><path d="M9.6 9.4a2.5 2.5 0 0 1 4.8.9c0 1.7-2.4 2.1-2.4 3.6M12 16.8h.01"/>'),
  monitor: svg('<rect x="3.5" y="4.5" width="17" height="11.5" rx="2"/><path d="M9 20h6M12 16v4"/>'),
  // 循环（小圈双箭头）与单次（中间带 1）。Repeat (two arrows in a loop) and once (with a 1).
  repeat: svg('<path d="M17 3.5 19.5 6 17 8.5"/><path d="M4.5 11.5V10a4 4 0 0 1 4-4h11"/><path d="M7 20.5 4.5 18 7 15.5"/><path d="M19.5 12.5V14a4 4 0 0 1-4 4h-11"/>'),
  repeatOnce: svg('<path d="M17 3.5 19.5 6 17 8.5"/><path d="M4.5 11.5V10a4 4 0 0 1 4-4h11"/><path d="M7 20.5 4.5 18 7 15.5"/><path d="M19.5 12.5V14a4 4 0 0 1-4 4h-11"/><path d="M11 10.5l1.3-.8V14.5" stroke-width="1.6"/>'),
  // 独占（单个实心圆：只放这一个）与叠加（两个交叠的圆）。
  // Exclusive (one solid circle: only this one) and overlap (two overlapping circles).
  exclusive: svg('<circle cx="12" cy="12" r="7" fill="currentColor" fill-opacity="0.22"/><circle cx="12" cy="12" r="2.8" fill="currentColor" stroke="none"/>'),
  overlap: svg('<circle cx="9" cy="12" r="5.5"/><circle cx="15" cy="12" r="5.5"/>'),
  headphones: svg('<path d="M4 14.5V12a8 8 0 0 1 16 0v2.5"/><rect x="3.5" y="14" width="4" height="6.5" rx="1.5"/><rect x="16.5" y="14" width="4" height="6.5" rx="1.5"/>'),
  // 虚拟麦克风（发送给游戏）：麦克风 + 向右的箭头。Virtual mic (sent to games): mic + arrow.
  send: svg('<rect x="5" y="3.5" width="5.5" height="10" rx="2.75"/><path d="M2.5 11a5.25 5.25 0 0 0 10.5 0M7.75 16.5v4"/><path d="M15 12h6.5M18.5 9l3 3-3 3"/>'),
  warning: svg('<path d="M12 4 21 19.5H3z"/><path d="M12 10v4.5M12 17h.01"/>'),
  // 地球（音效资源网站）与铅笔（编辑）。Globe (sound sites) and pencil (edit).
  globe: svg('<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.4 2.4 3.5 5.3 3.5 8.5s-1.1 6.1-3.5 8.5c-2.4-2.4-3.5-5.3-3.5-8.5s1.1-6.1 3.5-8.5z"/>'),
  // 眼睛（预览）。Eye (preview).
  eye: svg('<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>'),
  edit: svg('<path d="M14.5 5.5l4 4M4.5 19.5l1-4.5L15.8 4.7a1.4 1.4 0 0 1 2 0l1.5 1.5a1.4 1.4 0 0 1 0 2L9 18.5z"/>'),
  external: svg('<path d="M13.5 5.5h5v5M18.5 5.5 11 13M17 14v4.5a1 1 0 0 1-1 1H6.5a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1H11"/>'),
};
