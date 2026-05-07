#!/usr/bin/env node

import fs from 'fs';
import path from 'path';
import { spawnSync } from 'child_process';
import { fileURLToPath } from 'url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const TARGET_DIRS = ['scripts', 'renderer/src'];
const TARGET_FILES = ['vite.config.mjs'];
const CHECK_EXTENSIONS = new Set(['.js', '.cjs', '.mjs']);
const HYGIENE_FILES = [
  'package.json',
  'package-lock.json',
  'README.md',
  'CONTRIBUTING.md',
  'AGENTS.md',
  'LICENSE',
  'renderer/index.html',
  'vite.config.mjs'
];
const HYGIENE_DIRS = ['.github', 'renderer/src', 'scripts', 'docs', 'src-tauri'];
const HYGIENE_EXTENSIONS = new Set(['.js', '.cjs', '.mjs', '.jsx', '.css', '.html', '.md', '.yml', '.yaml', '.json', '.svg', '.rs', '.toml']);
const IGNORED_DIR_NAMES = new Set(['node_modules', 'dist', 'target', 'release', 'gen']);
const FORBIDDEN_PATTERNS = [
  { label: 'real-looking API key', pattern: /sk-[A-Za-z0-9_-]{20,}/ },
  { label: 'local user path', pattern: /C:\\Users\\|\/Users\/[A-Za-z0-9._-]+/i },
  { label: 'local project path', pattern: /E:\\Project\\/i }
];

function collectFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (IGNORED_DIR_NAMES.has(entry.name)) continue;
      files.push(...collectFiles(fullPath));
      continue;
    }
    if (entry.isFile() && CHECK_EXTENSIONS.has(path.extname(entry.name))) {
      files.push(fullPath);
    }
  }

  return files;
}

const files = [
  ...TARGET_FILES.map(file => path.join(ROOT, file)),
  ...TARGET_DIRS.flatMap(dir => collectFiles(path.join(ROOT, dir)))
].filter(file => fs.existsSync(file));

for (const file of files) {
  const result = spawnSync(process.execPath, ['--check', file], {
    cwd: ROOT,
    stdio: 'inherit'
  });
  if (result.status !== 0) {
    process.exit(result.status || 1);
  }
}

function collectHygieneFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (IGNORED_DIR_NAMES.has(entry.name)) continue;
      files.push(...collectHygieneFiles(fullPath));
      continue;
    }
    if (entry.isFile() && HYGIENE_EXTENSIONS.has(path.extname(entry.name))) {
      files.push(fullPath);
    }
  }

  return files;
}

const hygieneFiles = [
  ...HYGIENE_FILES.map(file => path.join(ROOT, file)),
  ...HYGIENE_DIRS.flatMap(dir => collectHygieneFiles(path.join(ROOT, dir)))
].filter(file => fs.existsSync(file) && path.basename(file) !== 'check.mjs');

let hygieneFailed = false;
for (const file of hygieneFiles) {
  const text = fs.readFileSync(file, 'utf8');
  for (const rule of FORBIDDEN_PATTERNS) {
    if (!rule.pattern.test(text)) continue;
    const relativePath = path.relative(ROOT, file);
    console.error(`Open-source hygiene check failed: ${rule.label} in ${relativePath}`);
    hygieneFailed = true;
  }
}

if (hygieneFailed) process.exit(1);

console.log(`Checked ${files.length} JavaScript files and ${hygieneFiles.length} hygiene files.`);
