# K.md Brand Guide

## Identity

**Display name:** K.md
**Tagline:** kausing much damage
**npm package:** @getforma/kmd
**CLI command:** kmd
**Full tagline (marketing):** Kausing Much Damage to dev workflow chaos
**Parent brand:** by GetForma

### Cultural Reference
KMD (Kausing Much Damage) was a 1990s hip-hop trio — the origin group of MF DOOM. The name is a respectful nod, not a theme. The tribute lives in three places only: the name, the tagline, and the gold accent color. Everything else is pure dev tool. No hip-hop imagery, no graffiti fonts, no literal visual references.

---

## Wordmark

The wordmark uses three distinct weights in monospace:

```
K  → Bold 700, #ebdbb2 (primary text)
.  → Bold 700, #d79921 (gold accent)
md → Regular 400, #a89984 (muted)
```

The K anchors with authority. The gold dot draws the eye and creates visual rhythm. The md fades back — it's there but doesn't compete. The period creates a beat: K · md.

**Sizes:**
- Display/hero: 48-64px
- UI header: 18px
- Inline/CLI: 13-15px

**Package name:** `@getforma/kmd` (no dot — npm doesn't allow dots)
**CLI:** `kmd` (lowercase, no dot)
**Spoken:** "K dot md"

---

## Color Palette

### Core — Gruvbox Dark

| Token    | Hex       | Usage                        |
|----------|-----------|------------------------------|
| bg0      | `#1d2021` | Page background              |
| bg0-s    | `#282828` | Surface / card background    |
| bg1      | `#32302f` | Elevated surface / hover row |
| bg2      | `#3c3836` | Borders, dividers            |
| bg3      | `#504945` | Subtle borders, separators   |
| bg4      | `#665c54` | Disabled / inactive          |
| fg0      | `#ebdbb2` | Primary text                 |
| fg1      | `#d5c4a1` | Secondary text               |
| fg2      | `#a89984` | Muted text                   |
| fg3      | `#928374` | Hint / disabled text         |

### Accent — Gold Ramp

Gold is the **only** branded color. It carries all interactive and branded elements.

| Token   | Hex       | Usage                              |
|---------|-----------|------------------------------------|
| bright  | `#fabd2f` | Hover / focus state                |
| primary | `#d79921` | Active tab, star, label, dot in logo |
| deep    | `#b57614` | Pressed state                      |
| muted   | `#8f5e0a` | Border accent                      |
| warm    | `#d65d0e` | Warning / secondary accent only    |

### Status Colors (Functional Only)

| Token    | Hex       | Usage                            |
|----------|-----------|----------------------------------|
| running  | `#b8bb26` | Port active, process alive       |
| error    | `#fb4934` | Kill button, stderr, crash       |
| info     | `#83a598` | Links, file paths, URLs          |
| inactive | `#665c54` | Port free, disabled state        |

**Rule:** Green, red, and blue are never used for branding — only for functional status indication.

---

## Typography

### Font Stack

```css
/* Brand font — used for wordmark, tabs, labels, headers, code, terminal */
--font-mono: 'JetBrains Mono', 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;

/* Utility font — used for body text and descriptions only */
--font-sans: 'Inter', system-ui, -apple-system, sans-serif;
```

**Mono is the brand font.** This is the opposite of most dev tools. Mono carries the identity — tabs, labels, section headers, metadata, terminal output. Sans only appears for body text and descriptions.

### Type Scale

| Role            | Font  | Size | Weight | Color    | Extra                    |
|-----------------|-------|------|--------|----------|--------------------------|
| Display         | Mono  | 48px | 700    | #ebdbb2  | Wordmark only            |
| Page heading    | Sans  | 22px | 500    | #ebdbb2  |                          |
| Section title   | Sans  | 16px | 500    | #ebdbb2  |                          |
| Body text       | Sans  | 14px | 400    | #d5c4a1  | line-height: 1.6         |
| UI label        | Mono  | 13px | 500    | #d79921  | letter-spacing: 0.04em   |
| Section header  | Mono  | 11px | 400    | #928374  | uppercase, letter-spacing: 0.1em |
| Code / terminal | Mono  | 12px | 400    | #a89984  |                          |
| Metadata        | Mono  | 11px | 400    | #665c54  |                          |
| Search highlight| Sans  | 14px | 400    | #a89984  | mark: bg #d79921, text #1d2021 |

---

## Component Tokens

### Navigation Tabs
- Active: `background: #d79921; color: #1d2021; font-weight: 500;`
- Inactive: `background: #32302f; color: #928374; font-weight: 400;`
- Font: Mono, 11px, letter-spacing: 0.04em, uppercase
- Border-radius: 4px
- Padding: 7px 16px

### Buttons
- **Primary outline:** `border: 0.5px solid #d79921; color: #d79921;`
- **Primary filled:** `background: #d79921; color: #1d2021; font-weight: 500;`
- **Destructive:** `background: #fb4934; color: #1d2021; font-weight: 500;`
- **Ghost:** `border: 0.5px solid #504945; color: #928374;`
- Font: Sans, 12px
- Border-radius: 4px
- Padding: 7px 14px

### Port Status Row
- Background: #32302f, border-radius: 4px, padding: 8px 12px
- Status dot: 8px circle, #b8bb26 (active) or #504945 (free)
- Port number: Mono 12px #ebdbb2
- Process name: Sans 12px #928374
- PID: Mono 10px #665c54
- Kill button: bg #fb4934, text #1d2021, 10px, 3px radius

### File Tree Item
- Selected: `background: #3c3836; border-left: 2px solid #d79921;`
- Normal: no background, no border
- Starred: gold star icon #d79921
- Filename: Sans 13px, #ebdbb2 (selected) or #a89984 (normal)
- Size: Mono 10px #665c54

### Search Result
- Container: `background: #32302f; border-left: 2px solid #d79921; border-radius: 4px;`
- File path: Mono 11px #83a598
- Snippet: Sans 13px #a89984
- Highlight mark: `background: #d79921; color: #1d2021; padding: 1px 4px; border-radius: 2px;`

---

## Design Rules

| Rule       | Value                                                                 |
|------------|-----------------------------------------------------------------------|
| Corners    | 4px on buttons, inputs, pills. 8px on cards. 0px on border-left accents. |
| Borders    | 0.5px solid #504945 default. 2px solid #d79921 for active/selected left accent. |
| Shadows    | None. Ever. Flat surfaces only.                                       |
| Spacing    | 8px inner gaps, 12px between related items, 20px between sections.    |
| Icons      | Lucide, 16px, stroke-width 1.5, #928374 default, #d79921 active. No filled icons. |
| One accent | Gold (#d79921) is the only branded color. Green/red/blue are functional only. |

---

## CLI Startup Banner

```
K.md  v0.1.0
kausing much damage
──────────────────────────
Docs ······ 340 files indexed
Scripts ··· 39 packages found
Ports ····· scanning...

→ http://localhost:4444
```

ANSI colors:
- "K" + version: bold white
- ".": bold yellow/gold
- "md": dim white
- "kausing much damage": yellow/gold
- Separator: dim
- Labels (Docs/Scripts/Ports): gray
- Dot leaders: dim
- Values: white
- "scanning...": green
- URL: cyan/blue

---

## Landing Page Copy

**Hero:**
```
K.md
by GetForma

kausing much damage to dev workflow chaos

Docs, scripts, and ports. One binary. One command. Zero config.

npx @getforma/kmd
```

**Three pillars:**
```
DOCS        SCRIPTS       PORTS
Search,     Run,          Monitor,
render,     stream,       manage,
mermaid     kill          clean
```

---

## What NOT to Do

- No hip-hop imagery (boomboxes, turntables, spray paint, breakdancers)
- No graffiti or decorative fonts
- No gradients or shadows
- No rounded corners above 8px
- No colored backgrounds on containers (dark only)
- No multiple accent colors (gold is the only one)
- No filled icons (outline only)
- No "fun" loading animations (keep them functional)
- The tribute is the name, the tagline, and the gold. That's it.
