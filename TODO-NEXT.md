# Next Tasks — Post-Tunnel Cleanup

These three tasks were identified during the cloudflare tunnel feature build (2026-03-27). Once all three are complete, delete this file.

---

## 1. Fix FormaJS `createShow` — missing else branch crashes

**Repo:** `FormaStack/formajs` (at `/Users/victorvillacis/dev/forma/getforma-dev/FormaStack/formajs`)

**Problem:** `createShow(condition, thenFn)` without a third argument crashes with `"Cannot access 'R' before initialization"` in minified builds. Every developer will try the two-argument pattern — it should work.

**Root cause:** `createShow` requires `(condition, thenFn, elseFn)` — all three arguments. When the else branch is omitted, the internal reactive system tries to call `undefined` as a function during cleanup/swap, which crashes.

**Important finding from code review:** FormaJS's `h()` function DOES handle null children correctly (line 423 of core: `if (child == null || child === false || child === true)`). The crash is specifically in `createShow`, NOT in `h()`. So `h('div', null, someChild, null, anotherChild)` is safe. But `createShow(cond, fn)` without the else branch is not.

**Fix:** In the `createShow` function, default the else branch:
```typescript
// Before:
export function createShow(when, thenFn, elseFn) { ... }

// After:
export function createShow(when, thenFn, elseFn = () => null) { ... }
```

Then verify `h()` handles the `null` return from the default else branch (it already does per the CR finding).

**After fixing:**
- Rebuild formajs (`npm run build` in the formajs directory)
- Publish new version
- Update `@getforma/core` in kmd's `package.json`
- kmd can then simplify all `createShow` calls that have `() => h('span', { style: 'display: none;' })` as the else branch — those are workarounds for this bug

**Tests to add:** `createShow(signal, thenFn)` with two arguments should render nothing when condition is false, render thenFn when true, and not crash.

---

## 2. Move starred files from localStorage to SQLite (kmd)

**Repo:** `kmd` (this repo)

**Problem:** Starred files are stored in the browser's `localStorage`. This means:
- They're browser-specific (different browser = different stars)
- They don't show through the tunnel (tunnel visitor has different origin = different localStorage)
- They're inconsistent with bookmarks and annotations which are in SQLite
- In ephemeral mode, localStorage stars survive but DB bookmarks don't — confusing

**What to do:**

1. **Add table in `src/db.rs`:**
```sql
CREATE TABLE IF NOT EXISTS doc_stars (
    id INTEGER PRIMARY KEY,
    root TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(root, file_path)
);
```

2. **Add API endpoints in `src/server.rs`:**
- `GET /api/docs/stars` — list starred file paths
- `POST /api/docs/stars` — star a file `{ root, file_path }`
- `DELETE /api/docs/stars/{id}` — unstar

3. **Update `client/components/FileTree.ts`:**
- Currently `getStarredPaths()` and `toggleStar()` use localStorage
- Replace with API calls (`kmdFetch`)
- The star state should be a signal fetched from the server

4. **Update tunnel allowlist in `src/server.rs`:**
- Add `GET /api/docs/stars` to `is_allowed_for_tunnel()` (read-only through tunnel)
- POST/DELETE remain blocked for tunnel visitors

5. **Migration:** On first run, optionally migrate existing localStorage stars to the DB (nice-to-have, not required — users will just re-star).

**Files to modify:** `src/db.rs`, `src/server.rs`, `client/components/FileTree.ts`, `client/pages/DocsPage.ts` (bookmarks panel reads stars)

---

## 3. Add ephemeral mode warning (kmd)

**Repo:** `kmd` (this repo)

**Problem:** When a user runs `kmd` (ephemeral mode), all DB data (bookmarks, annotations, stars, script notes) is stored in a temp directory that's deleted on exit. The user doesn't know this until they restart and everything is gone.

**What to do:**

In `src/main.rs`, where the startup banner prints, add a line for ephemeral mode:

```
  K.md  v0.1.7
  kausing much damage
  -------------------------------
  Mode ······ ephemeral ⚠ data won't persist — use `kmd create <name>` for workspaces
  Docs ······ 3 files indexed
  ...
```

Or more subtle:
```
  Mode ······ ephemeral (session only)
```

The key information: the user should know their bookmarks/annotations/stars will be lost when they exit. The fix is `kmd create <name>` to create a persistent workspace.

**File to modify:** `src/main.rs` — the banner print section (search for "Mode" or "ephemeral")

---

## Order of Operations

1. **FormaJS fix first** — this unblocks cleaner code in kmd (remove the `() => h('span', { style: 'display: none;' })` workarounds)
2. **Starred files to SQLite** — requires DB migration, API endpoints, frontend changes
3. **Ephemeral warning** — one-line change in main.rs

## Delete This File

Once all three tasks are complete and merged to main, delete this file.
