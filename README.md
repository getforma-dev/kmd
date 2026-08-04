# K.md

[![npm](https://img.shields.io/npm/v/@getforma/kmd)](https://www.npmjs.com/package/@getforma/kmd)
[![CI](https://github.com/getforma-dev/kmd/actions/workflows/release.yml/badge.svg)](https://github.com/getforma-dev/kmd/actions/workflows/release.yml)
[![Socket Badge](https://socket.dev/api/badge/npm/package/@getforma/kmd)](https://socket.dev/npm/package/@getforma/kmd)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Kausing Much Damage** to dev workflow chaos.

K.md is a local-first developer dashboard for navigating, searching, and annotating markdown documentation across your monorepo. It bundles a Rust server with a reactive TypeScript frontend into a single binary — no config, no cloud, just `kmd`.

## Features

- **Markdown explorer** — File tree with full-text search across all `.md` files in your workspace
- **Syntax highlighting** — Code blocks rendered with Syntect, mermaid diagrams with client-side rendering
- **Text highlighting & annotations** — Select text, pick a color, add notes. Highlights persist across sessions (SQLite)
- **Table of contents** — Auto-generated "On this page" sidebar with scroll tracking and heading bookmarks
- **Script runner** — Discover and run package.json scripts with real-time output streaming via WebSocket
- **Port monitor** — Live scan of active ports with process info and one-click kill
- **Terminal** — Embedded PTY terminal sessions
- **Share** — One click publishes a read-only docs view over a Cloudflare tunnel, so you can read your docs from a phone or send a teammate a link ([details](#sharing))
- **Mobile layout** — Phones and tablets get a dedicated layout with a bottom tab bar and slide-over file tree
- **Multi-root workspaces** — Point kmd at a monorepo and it discovers all documentation roots
- **Offline** — Everything runs locally. No network required until you choose to share.

## Install

```bash
npx @getforma/kmd
```

Or install globally:

```bash
npm i -g @getforma/kmd
kmd
```

## Usage

Run from any directory containing markdown files:

```bash
kmd
```

kmd starts a local server (default port 4444) and opens your browser. It recursively discovers all `.md` files from the current directory.

### Options

```
kmd                     # Start in current directory, open browser
kmd --port 3000         # Use a custom port
kmd --no-open           # Start without opening browser
kmd --force             # Force start even if port is in use
```

### Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| `Cmd/Ctrl + K` | Focus search |
| `Cmd/Ctrl + Shift + H` | Highlight selected text with last-used color |
| `Escape` | Close toolbar / dismiss |

## Sharing

Click **Share** in the sidebar and kmd opens a [Cloudflare quick tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/) to your local server, giving you a public HTTPS URL like `https://four-random-words.trycloudflare.com`. Open it on your phone, or send it to someone. Click again to stop.

`cloudflared` is downloaded automatically on first use to `~/.kmd/bin/` — nothing to install. If your machine can't reach GitHub's release CDN, drop the binary in that directory yourself and kmd will use it.

### What visitors can and cannot see

**Shared links are docs-only.** kmd is not just a docs viewer — it runs an embedded PTY, a script runner, and a port monitor. None of that is reachable through the tunnel. A server-side allowlist permits only documentation reads; every other route returns 403, and the tunnel view hides the tabs that would hit them.

| Through the tunnel | |
|---|---|
| Docs, search, table of contents, annotations, git status | Visible |
| Terminal, scripts, ports, processes, env files | Blocked (403) |
| Starting or stopping the tunnel itself | Blocked — localhost only |

The WebSocket feed is filtered the same way: visitors receive documentation events only, never process output or port scans.

### What the URL is worth

**The URL is the only secret.** There is no login. The subdomain is four random words, unguessable in practice and not enumerable by Cloudflare — but anyone who has the link can read the docs you shared, and a new URL is generated every time you start sharing. Treat a shared link like a secret gist: fine for your own phone or a trusted teammate, not for anything confidential.

Authenticated sharing — stable personal URLs, login-gated access, and full remote access including the terminal — is planned via [GateWASM](https://auth.getforma.dev/platform/onboarding).

## Development

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) >= 16

### Setup

```bash
npm install
```

### Dev mode

```bash
npm run dev
```

Builds the client and starts the Rust server with hot-reload.

### Build for release

```bash
npm run build
```

### Run tests

```bash
npm test
```

Runs the Playwright E2E test suite (34 tests covering app shell, markdown explorer, script runner, port monitor, security, and more).

### Link locally

```bash
npm run link
```

Symlinks the built binary to `~/.local/bin/kmd` for global access.

## Architecture

```
client/          TypeScript frontend (@getforma/core reactive framework)
  pages/         Page components (DocsPage, ScriptsPage, PortsPage, TerminalPage)
  components/    Reusable components (FileTree, SearchBar)
  styles/        Gruvbox dark/light theme CSS
  lib/           Utilities (security, mermaid, logging)

src/             Rust backend (Axum web framework)
  server.rs      HTTP/WS routes, static file serving, CSRF protection
  db.rs          SQLite schema (annotations, bookmarks, FTS)
  services/      Markdown rendering, port scanning, env parsing

tests/e2e/       Playwright end-to-end tests
npm/             Platform-specific binary packages for npm distribution
```

## License

MIT — see [LICENSE](LICENSE).
