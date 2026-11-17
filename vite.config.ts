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
    coverage: {
      provider: 'v8',
      reporter: ['text', 'html', 'lcov'],
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['src/**/*.test.{ts,tsx}', 'src/vite-env.d.ts', 'src/globals.d.ts', 'src/main.tsx'],
      thresholds: {
        branches: 80,
        functions: 80,
        lines: 80,
        statements: 80,
      },
    },
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
  },
});
