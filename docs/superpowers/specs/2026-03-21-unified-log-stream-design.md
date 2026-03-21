# Unified Log Stream + Crash Badge

## Goal

Turn kmd from "a dashboard with tabs" into the place you look when something goes wrong across services. Two features that solve two different moments:

1. **"All" tab** — I'm on the Scripts tab and want to see everything at once
2. **Crash badge** — I'm on another tab and don't know something died

No backend changes. All data already flows over WebSocket.

---

## 1. "All" Virtual Tab

### Location

First tab in the Scripts output panel process tabs bar, before any per-process tabs. Label: `All (3)` where 3 is the count of active processes.

### Behavior

When selected, shows all stdout/stderr from all running processes interleaved chronologically. Each line is prefixed with a colored label identifying the source process.

### Line Format

```
[dev :4500]   Starting development server...
[dev :4500]   Listening on http://localhost:4500
[build]       dist/client/app.js  447KB
[start :4501] K.md v0.1.0
```

Label text matches the tab label (scriptName + port if assigned). Label is a styled `<span>` before the ANSI-rendered content. Label color cycles per process through a fixed palette: green, blue, magenta, cyan, yellow (Gruvbox colors).

### Data Flow

- No backend changes
- WS messages already arrive with `process_id` on every stdout/stderr line
- Today: `handleWsMessage` routes each line to `processOutputMap[pid]`
- New: also append to `allOutputLines` array with `{ ...line, processId, label, color }`
- The Terminal component renders `allOutputLines` when "All" tab is selected — same component, different lines source
- Clicking a label in the "All" view switches to that process's individual tab

### Buffer Cap

`allOutputLines` uses the same `MAX_OUTPUT_LINES` (5000) constant. When exceeded, splice from the front. Three chatty processes dumping to one array will hit this fast.

### Filter Toggles

Small clickable process name pills above the "All" stream to show/hide specific processes. Each pill shows the process label + color dot. Toggling a pill filters `allOutputLines` by process_id without removing data (just display filter).

### Files Changed

- `client/pages/ScriptsPage.ts` — new `allOutputLines` array, populate in `handleWsMessage`, add "All" tab to `ProcessTabs`, filter UI, label rendering
- `client/components/Terminal.ts` — support optional `labelFn` prop that prepends colored labels to each line (only used in "All" mode)

---

## 2. Crash Badge on Scripts Tab

### Behavior

When a managed process exits with non-zero code while the user is NOT on the Scripts tab, the Scripts nav item shows a red badge with the crash count. Badge clears when user navigates to Scripts.

### Intentional Kill Tracking

Kills initiated from the UI (Ports tab Stop button, Scripts tab Kill button, tab close button) set a flag on the process before sending the kill request. The exit handler checks this flag — if set, the exit is intentional and does NOT count toward the crash badge.

Implementation: `intentionalKills` Set in ScriptsPage stores process IDs that were deliberately stopped. When an `exit` message arrives with non-zero code, check if `pid` is in `intentionalKills` — if yes, remove it and skip the badge increment. If no, increment crash count.

The kill from PortsPage (Stop button for managed processes) also needs to register — it calls `/api/processes/:id/kill` with the process_id, so we need a way for PortsPage kills to register in the same Set. Solution: the `intentionalKills` Set lives at the app level (passed down via props or a shared module), not inside ScriptsPage.

### Data Flow

- `app.ts` owns a `crashCount` signal and an `intentionalKills` Set
- The root WS handler in `app.ts` listens for `exit` messages with non-zero codes
- If `process_id` is NOT in `intentionalKills`, increment `crashCount`
- If it IS in `intentionalKills`, remove it from the Set (cleanup)
- Successful exits (code 0) and null-code exits (killed) with intentional flag are ignored
- `crashCount` signal is passed to Sidebar, which renders a red badge when > 0
- Navigating to `#scripts` resets `crashCount` to 0
- ScriptsPage and PortsPage both add to `intentionalKills` before sending kill requests

### Badge Design

Small red circle with count, positioned on the Scripts nav item. Matches the Gruvbox red (`#fb4934`). If count is 1, just show the dot without a number.

### Files Changed

- `client/app.ts` — `crashCount` signal, `intentionalKills` Set, WS exit handler, pass to Sidebar and pages
- `client/components/Sidebar.ts` — accept `crashCount` prop, render red badge on Scripts item
- `client/pages/ScriptsPage.ts` — add to `intentionalKills` on kill actions, clear crashCount on mount
- `client/pages/PortsPage.ts` — add to `intentionalKills` on managed process Stop

---

## Implementation Order

1. "All" tab (biggest feature, most code)
2. Crash badge (smaller, depends on understanding the WS flow from #1)

## Out of Scope

- Log persistence (write to file)
- Log search/filter by text content
- Log level filtering (info/warn/error)
- Process grouping by workspace root
