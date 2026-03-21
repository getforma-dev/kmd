# KMD Polish Pass — Scripts Page + Quick Fixes

## Goal

Make what exists feel finished. No new features. Fix the feedback gaps in the Scripts page (the most interactive surface), and address a handful of cross-app rough edges.

---

## 1. Running Indicator on Script Buttons

### Problem

When you click a script button (e.g. "dev"), nothing changes on the button itself. You can't tell at a glance which scripts are active. It's easy to accidentally start the same script twice.

### Design

New `Map<string, string>` called `activeScriptMap` keyed by `root:packagePath:scriptName` → `processId`. This is the true "is this script running" state (the existing `runningScripts` Set is just a 2-second debounce timer, not actual running state).

- Populated in `runScript()` when the API response arrives with a process_id
- Cleared in `handleWsMessage()` when an `exit` message arrives (lookup processId in the map values)
- The `PackageCard` `createEffect` checks this map to determine button state

When a script is running:

- The button gets a pulsing green dot (same `status-dot active` used in process tabs)
- The button is disabled with reduced opacity
- Script name is stored in a `data-script` attribute on the button (not read from `textContent`, since that would break if we append status text)
- On process exit, the button returns to its default state

### Files Changed

- `client/pages/ScriptsPage.ts` — new `activeScriptMap`, populate in `runScript`, clear on exit, read in `PackageCard` effect. Store script name in `data-script` attribute instead of reading `btn.textContent`.

---

## 2. ANSI Color Support in Script Output

### Problem

Terminal.ts strips all ANSI escape codes via `ANSI_REGEX`. Build tools (cargo, tsc, vite), test runners (jest, vitest), and dev servers all use ANSI colors for errors (red), warnings (yellow), success (green), and emphasis (bold). Stripping them to plain monochrome loses critical visual information.

### Design

Replace the strip-and-textContent approach with an ANSI-to-HTML renderer. Parse ANSI SGR sequences (the `\x1b[...m` codes) and emit `<span>` elements with inline styles or CSS classes.

**Supported codes (covering 99% of real-world output):**
- Reset (0)
- Bold (1), dim (2), italic (3), underline (4)
- Foreground colors: 30-37 (standard), 90-97 (bright)
- Background colors: 40-47 (standard), 100-107 (bright)
- 256-color mode: `38;5;N` / `48;5;N`

**Not supported (rare, not worth the complexity):**
- 24-bit true color (`38;2;R;G;B`)
- Cursor movement, erase sequences (not relevant for line-by-line output)

**Color mapping:** Use Gruvbox dark palette colors for the 16 standard ANSI colors. These must be hardcoded (not CSS variables that flip with theme), because the terminal background is always dark (`#1d2021`). CSS classes are scoped under `.terminal` to avoid conflicts.

**Unsupported sequences:** Any ANSI sequence not in the supported set is silently stripped (same as current behavior). Never rendered as raw escape text.

**Implementation:** A function `renderAnsiLine(text: string): HTMLElement` that:
1. Splits the line on ANSI escape sequences
2. Tracks current style state (color, bold, etc.)
3. Emits `<span>` elements with appropriate styles
4. Returns a `<div class="terminal-line {type}">` with the spans inside

This replaces the current `lineEl.textContent = stripAnsi(line.text)` in Terminal.ts.

### Files Changed

- `client/components/Terminal.ts` — replace `stripAnsi` + `textContent` with `renderAnsiLine`
- `client/styles/dev.css` — add ANSI color classes under `.terminal` scope with hardcoded dark Gruvbox colors

---

## 3. Show Assigned Port on Running Script Cards

### Problem

When kmd assigns PORT=4500 to a script, you only see it in the terminal output pane. The package card in the script list doesn't reflect which port was assigned. You have to switch to the Ports tab to find out.

### Design

When a script is running and has an assigned port, show the port inline on the package card — right next to the running indicator.

Format: `dev · :4500` (or `dev · :4500 (Vite)` if framework was detected).

The assigned port info is already returned by `/api/scripts/run` and stored in `processLabelMap`. We just need to surface it in the card's button state, not just the tab label.

### Data Flow

- `runScript()` already stores `port` from the API response
- Store port in a `Map<string, string>` called `runningPortMap` keyed by `root:packagePath:scriptName` → port string (e.g. "4500")
- `PackageCard` reads this map in its reactive effect alongside `activeScriptMap`
- On process exit, clear the entry. The exit handler receives `process_id`, so `processMetaMap` needs a `rootPath` field added to enable reverse lookup from processId → composite key for cleanup.

### Files Changed

- `client/pages/ScriptsPage.ts` — new `runningPortMap`, add `rootPath` to `processMetaMap` entries, populate on script run, clear on exit, read in `PackageCard`

---

## 4. Auto-Refresh Ports After Kill

### Problem

After clicking Kill on a port and getting the success state, the port list doesn't update. You have to click "Scan now" manually.

### Design

Already half-implemented: `killPort()` in PortsPage.ts calls `scanNow()` after a successful kill (line 163: `setTimeout(() => scanNow(), 500)`). But `scanNow` hits `POST /api/ports/scan` which broadcasts raw un-enriched data.

Fix: after a successful kill, call `fetchPorts()` (the enriched GET endpoint) instead of `scanNow()`. This also picks up the managed badge correctly. The 500ms delay is best-effort — if the OS hasn't released the port yet, the next periodic WS-triggered refresh (every 5s) will catch it.

### Files Changed

- `client/pages/PortsPage.ts` — replace `scanNow()` call in kill success handler with `fetchPorts()`

---

## 5. Minify app.js

### Problem

app.js is 624KB unminified. With one esbuild flag it drops to ~300KB.

### Design

Add `--minify` to the esbuild invocation in `build-client.sh`. No source maps needed for now (kmd is a local tool, errors are debuggable from the terminal output).

### Files Changed

- `build-client.sh` — add `--minify` flag

---

## 6. Help Panel Content Gaps

### Problem

Help panel doesn't mention:
- Focus mode (exists in DocsPage — hides file tree + TOC for distraction-free reading)
- Theme toggle (sun/moon button in sidebar footer)
- Command palette doc search (Cmd+K searches across all markdown files)

### Design

Add these to the existing Help panel sections:

- **Tips section:** Add bullet for focus mode ("Focus mode button in the Docs tab hides the file tree and TOC for distraction-free reading"). Note: focus mode has no keyboard shortcut — it's a UI button only.
- **Tips section:** Add bullet for command palette doc search ("Cmd+K also searches across all docs, not just pages and actions")
- **Tips section:** Add bullet for theme toggle ("Toggle dark/light mode from the sidebar")

### Files Changed

- `client/components/HelpPanel.ts` — add rows to Keyboard Shortcuts and Tips sections

---

## Implementation Order

1. ANSI color support (biggest visual impact, touches Terminal.ts which everything uses)
2. Running indicator on script buttons (most frequent interaction)
3. Show assigned port on cards (builds on #2's state)
4. Auto-refresh ports after kill (one-line fix)
5. Minify app.js (one-line fix)
6. Help panel gaps (content only)

## Out of Scope

- Light theme terminal fix (needs deeper xterm.js theme work)
- Keyboard file tree navigation (new feature, not polish)
- Stars/favorites (new feature)
- E2E test expansion (separate effort)
- Split-pane terminals (new feature)
