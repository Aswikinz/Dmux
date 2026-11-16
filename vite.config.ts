import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

// Tauri expects a fixed dev port (tauri.conf.json devUrl).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    target: 'es2022',
    outDir: 'dist',
  },
  test: {
    environment: 'jsdom',
    // Unit tests live in src/; e2e/ specs run under WebdriverIO, not Vitest.
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
