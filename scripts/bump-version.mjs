#!/usr/bin/env node
// Bump the release version everywhere it appears, in one step.
//
// Seven files have to agree — Cargo.toml, Cargo.lock, and six npm packages
// (including the meta package's optionalDependencies, which pin exact
// versions). The release workflow only checks them at tag time, so a missed
// file used to surface as a failed or half-finished publish: 0.1.5 shipped all
// five platform packages with no meta package and had to be abandoned for
// 0.1.6.
//
//   npm run bump 0.2.1
//   npm run bump -- --check     # verify consistency, change nothing

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');

const PLATFORM_PKGS = [
  'kmd-darwin-arm64',
  'kmd-darwin-x64',
  'kmd-linux-x64',
  'kmd-linux-arm64',
  'kmd-win32-x64',
];
const ALL_PKGS = ['kmd', ...PLATFORM_PKGS];

const read = (p) => readFileSync(join(ROOT, p), 'utf8');
const write = (p, s) => writeFileSync(join(ROOT, p), s);
const pkgPath = (name) => `npm/${name}/package.json`;

/** The `kmd` stanza in Cargo.lock. Tolerates CRLF checkouts. */
const LOCK_RE = /(\[\[package\]\]\r?\nname = "kmd"\r?\nversion = )"([^"]+)"/;

/** Current version according to the published meta package. */
function currentVersion() {
  return JSON.parse(read(pkgPath('kmd'))).version;
}

/** Every place a version lives, as {file, found} pairs. */
function collect() {
  const out = [];

  const cargo = read('Cargo.toml').match(/^version\s*=\s*"([^"]+)"/m);
  out.push({ file: 'Cargo.toml', found: cargo?.[1] });

  // Only the `kmd` package stanza in the lockfile — other crates legitimately
  // share version numbers with us. `\r?\n` because the lockfile is CRLF on a
  // Windows checkout.
  const lock = read('Cargo.lock').match(LOCK_RE);
  out.push({ file: 'Cargo.lock', found: lock?.[2] });

  for (const name of ALL_PKGS) {
    out.push({ file: pkgPath(name), found: JSON.parse(read(pkgPath(name))).version });
  }

  const meta = JSON.parse(read(pkgPath('kmd')));
  for (const [dep, range] of Object.entries(meta.optionalDependencies ?? {})) {
    out.push({ file: `npm/kmd/package.json → ${dep}`, found: range });
  }

  return out;
}

function check() {
  const entries = collect();
  const versions = [...new Set(entries.map((e) => e.found))];
  for (const e of entries) {
    console.log(`  ${e.found ?? '(not found)'}  ${e.file}`);
  }
  if (versions.length === 1 && versions[0]) {
    console.log(`\nConsistent at ${versions[0]}.`);
    return 0;
  }
  console.error(`\nInconsistent: found ${versions.join(', ')}`);
  return 1;
}

function bump(next) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(next)) {
    console.error(`Not a semver version: ${next}`);
    return 1;
  }

  const prev = currentVersion();
  if (prev === next) {
    console.error(`Already at ${next}.`);
    return 1;
  }

  // Cargo.toml — the first `version =` is the package's own.
  write('Cargo.toml', read('Cargo.toml').replace(/^version\s*=\s*"[^"]+"/m, `version = "${next}"`));

  // Cargo.lock — only our own stanza.
  write('Cargo.lock', read('Cargo.lock').replace(LOCK_RE, `$1"${next}"`));

  // Edit the JSON textually rather than re-serialising it. Round-tripping
  // through JSON.stringify reformats these hand-written files (`["win32"]`
  // becomes three lines) and would rewrite line endings, burying a one-line
  // version change in unrelated churn.
  for (const name of ALL_PKGS) {
    const p = pkgPath(name);
    let text = read(p);
    // The package's own version is the first "version" key.
    text = text.replace(/("version"\s*:\s*)"[^"]+"/, `$1"${next}"`);
    // Platform pins in the meta package's optionalDependencies.
    text = text.replace(/("@getforma\/kmd-[a-z0-9-]+"\s*:\s*)"[^"]+"/g, `$1"${next}"`);
    write(p, text);
  }

  console.log(`Bumped ${prev} -> ${next}:\n`);
  const code = check();
  if (code === 0) {
    console.log(`\nNext: commit, then \`git tag v${next} && git push origin v${next}\`.`);
  }
  return code;
}

const arg = process.argv[2];
if (!arg || arg === '--check') {
  process.exit(check());
}
process.exit(bump(arg.replace(/^v/, '')));
