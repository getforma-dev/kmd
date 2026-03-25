# K.md

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
- **Multi-root workspaces** — Point kmd at a monorepo and it discovers all documentation roots
- **Offline** — Everything runs locally. No network required after install.

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

MIT
