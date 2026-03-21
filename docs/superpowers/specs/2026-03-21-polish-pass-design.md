# KMD Polish Pass — Scripts Page + Quick Fixes

## Goal

Make what exists feel finished. No new features. Fix the feedback gaps in the Scripts page (the most interactive surface), and address a handful of cross-app rough edges.

---

## 1. Running Indicator on Script Buttons

### Problem

When you click a script button (e.g. "dev"), nothing changes on the button itself. You can't tell at a glance which scripts are active. It's easy to accidentally start the same script twice.

### Design

Track running process IDs per script key (`root:packagePath:scriptName`). When a script is running:

- The button gets a pulsing green dot (same `status-dot active` used in process tabs) prepended to the label
- The button text changes from `dev` to `dev · running`
- The button is disabled (no double-starts, same debounce we already have but visual)
- On process exit, the button returns to its default state

The running state is derived from `activeProcesses` signal + `processExitMap` — no new state needed. The `PackageCard` function already subscribes to `runningVersion`; we just need to enhance the visual from "opacity: 0.4" to the proper indicator.

### Files Changed

- `client/pages/ScriptsPage.ts` — `PackageCard` function, button rendering inside the `createEffect`

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

**Color mapping:** Use Gruvbox palette colors for the 16 standard ANSI colors so output looks native to the theme.

**Implementation:** A function `renderAnsiLine(text: string): HTMLElement` that:
1. Splits the line on ANSI escape sequences
2. Tracks current style state (color, bold, etc.)
3. Emits `<span>` elements with appropriate styles
4. Returns a `<div class="terminal-line {type}">` with the spans inside

This replaces the current `lineEl.textContent = stripAnsi(line.text)` in Terminal.ts.

### Files Changed

- `client/components/Terminal.ts` — replace `stripAnsi` + `textContent` with `renderAnsiLine`
- `client/styles/dev.css` — add ANSI color classes mapped to Gruvbox palette

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
- Store port in a `Map<string, number>` keyed by `root:packagePath:scriptName`
- `PackageCard` reads this map in its reactive effect alongside `runningScripts`
- On process exit, clear the entry

### Files Changed

- `client/pages/ScriptsPage.ts` — new `runningPortMap`, populate on script run, read in `PackageCard`

---

## 4. Auto-Refresh Ports After Kill

### Problem

After clicking Kill on a port and getting the success state, the port list doesn't update. You have to click "Scan now" manually.

### Design

Already half-implemented: `killPort()` in PortsPage.ts calls `scanNow()` after a successful kill (line 163: `setTimeout(() => scanNow(), 500)`). But `scanNow` hits `POST /api/ports/scan` which broadcasts raw un-enriched data.

Fix: after a successful kill, call `fetchPorts()` (the enriched GET endpoint) instead of `scanNow()`. This also picks up the managed badge correctly.

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

- **Keyboard Shortcuts section:** Add row for focus mode toggle
- **Tips section:** Add bullet for command palette doc search ("Cmd+K searches docs, pages, and actions")
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
