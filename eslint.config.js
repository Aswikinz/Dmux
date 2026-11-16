import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      'target/**',
      'src-tauri/target/**',
      'src-tauri/gen/**',
      'coverage/**',
      '.wdio/**',
      'e2e/logs/**',
      'e2e/artifacts/**',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      globals: globals.browser,
    },
  },
  {
    files: ['e2e/**/*.ts', 'vite.config.ts', 'eslint.config.js'],
    languageOptions: {
      globals: globals.node,
    },
  },
);
