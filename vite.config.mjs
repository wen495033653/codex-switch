import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const MODULE_DIR = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root: path.resolve(MODULE_DIR, 'renderer'),
  plugins: [react()],
  build: {
    outDir: path.resolve(MODULE_DIR, 'renderer/dist'),
    emptyOutDir: true
  },
  base: './',
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true
  }
});
