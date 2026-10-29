// 预览窗口注入脚本：下载提示（「在浏览器中打开」在窗口自己的标题栏上）。
// Preview-window injected script: download notices ("open in browser" lives in the
// window's own title bar).
(() => {
  if (window.top !== window || window.__ferryMic) return;
  window.__ferryMic = true;
  const TOAST_MS = 4500;
  let root = null;
  let pending = [];

  const mount = () => {
    if (root || !document.body) return;
    const host = document.createElement('div');
    host.style.cssText = 'position:fixed;top:0;right:0;z-index:2147483647;';
    // Shadow DOM：不受网站样式影响。Shadow DOM: unaffected by the site's styles.
    root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = `
      <style>
        .toasts { position: fixed; top: 12px; right: 12px; display: flex; flex-direction: column;
          gap: 8px; align-items: flex-end; font: 13px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif; }
        .toast { max-width: 340px; padding: 10px 14px; border-radius: 12px; color: #eceef3;
          background: rgba(20, 22, 27, 0.92); box-shadow: 0 8px 24px rgba(0,0,0,.35);
          border-left: 3px solid #7b6cff; animation: in .18s ease-out; word-break: break-all; }
        .toast.ok { border-left-color: #3ddc97; }
        .toast.warn { border-left-color: #ffb454; }
        @keyframes in { from { opacity: 0; transform: translateY(-6px); } }
      </style>
      <div class="toasts"></div>`;
    document.documentElement.appendChild(host);
    pending.forEach(([t, k]) => window.__ferryToast(t, k));
    pending = [];
  };

  // 由 App 在下载开始 / 完成时调用。Called by the app when a download starts / finishes.
  window.__ferryToast = (text, kind) => {
    if (!root) {
      pending.push([text, kind]);
      return;
    }
    const box = root.querySelector('.toasts');
    const el = document.createElement('div');
    el.className = 'toast ' + (kind || '');
    el.textContent = text;
    box.appendChild(el);
    setTimeout(() => el.remove(), TOAST_MS);
  };

  if (document.body) mount();
  else document.addEventListener('DOMContentLoaded', mount);
})();
