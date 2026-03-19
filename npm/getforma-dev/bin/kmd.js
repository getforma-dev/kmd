#!/usr/bin/env node

// Platform detection script for @getforma/kmd
// Resolves the correct platform-specific binary and executes it

const { execFileSync } = require('child_process');
const { join } = require('path');

const PLATFORMS = {
  'darwin-arm64': '@getforma/kmd-darwin-arm64',
  'darwin-x64': '@getforma/kmd-darwin-x64',
  'linux-x64': '@getforma/kmd-linux-x64',
  'win32-x64': '@getforma/kmd-win32-x64',
};

const platformKey = `${process.platform}-${process.arch}`;
const pkg = PLATFORMS[platformKey];

if (!pkg) {
  console.error(
    `kmd does not have a prebuilt binary for ${process.platform}-${process.arch}.\n` +
    `Supported platforms: ${Object.keys(PLATFORMS).join(', ')}`
  );
  process.exit(1);
}

let binPath;
try {
  const pkgPath = require.resolve(`${pkg}/package.json`);
  const binName = process.platform === 'win32' ? 'kmd.exe' : 'kmd';
  binPath = join(pkgPath, '..', 'bin', binName);
} catch {
  console.error(
    `Failed to find the platform-specific package ${pkg}.\n` +
    `This usually means the optional dependency was not installed.\n` +
    `Try: npm install @getforma/kmd --force`
  );
  process.exit(1);
}

// Forward all arguments to the binary
try {
  execFileSync(binPath, process.argv.slice(2), { stdio: 'inherit' });
} catch (e) {
  if (e.status !== null) {
    process.exit(e.status);
  }
  throw e;
}
