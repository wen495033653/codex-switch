#!/usr/bin/env node

import path from 'path';
import { spawnSync } from 'child_process';
import { fileURLToPath } from 'url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const targetDir = path.join(ROOT, 'src-tauri', 'target');

const result = spawnSync('cargo', [
  'check',
  '--manifest-path',
  path.join(ROOT, 'src-tauri', 'Cargo.toml'),
  '--target-dir',
  targetDir
], {
  cwd: ROOT,
  stdio: 'inherit',
  shell: process.platform === 'win32'
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status || 0);
