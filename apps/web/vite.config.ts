import { defineConfig } from 'vite';

// SoundBridge 前端构建配置。
// SoundBridge front-end build config.
export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    // 开发时把 API 代理到后端（避免跨域）。
    // Proxy API to the backend during dev (avoid CORS).
    proxy: {
      '/api': 'http://127.0.0.1:8321',
    },
  },
});
