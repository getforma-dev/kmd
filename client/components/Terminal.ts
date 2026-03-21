import { h, createEffect } from '@getforma/core';

// ---------------------------------------------------------------------------
// ANSI → HTML renderer
// ---------------------------------------------------------------------------

// Matches any ANSI escape sequence (SGR, OSC, charset, mode)
const ANSI_SEQ = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07|\x1b[()][AB012]|\x1b[=>]/g;

// Gruvbox dark palette for ANSI standard colors (hardcoded — terminal bg is always dark)
const ANSI_FG: Record<number, string> = {
  30: '#282828', 31: '#cc241d', 32: '#98971a', 33: '#d79921',
  34: '#458588', 35: '#b16286', 36: '#689d6a', 37: '#a89984',
  90: '#928374', 91: '#fb4934', 92: '#b8bb26', 93: '#fabd2f',
  94: '#83a598', 95: '#d3869b', 96: '#8ec07c', 97: '#ebdbb2',
};

const ANSI_BG: Record<number, string> = {
  40: '#282828', 41: '#cc241d', 42: '#98971a', 43: '#d79921',
  44: '#458588', 45: '#b16286', 46: '#689d6a', 47: '#a89984',
  100: '#928374', 101: '#fb4934', 102: '#b8bb26', 103: '#fabd2f',
  104: '#83a598', 105: '#d3869b', 106: '#8ec07c', 107: '#ebdbb2',
};

// 256-color palette: 0-7 standard, 8-15 bright, 16-231 cube, 232-255 grayscale
function color256(n: number): string | null {
  if (n < 0 || n > 255) return null;
  if (n < 8) return ANSI_FG[30 + n] || null;
  if (n < 16) return ANSI_FG[90 + (n - 8)] || null;
  if (n < 232) {
    // 6x6x6 color cube
    const idx = n - 16;
    const r = Math.floor(idx / 36) * 51;
    const g = Math.floor((idx % 36) / 6) * 51;
    const b = (idx % 6) * 51;
    return `rgb(${r},${g},${b})`;
  }
  // Grayscale ramp
  const v = 8 + (n - 232) * 10;
  return `rgb(${v},${v},${v})`;
}

interface AnsiState {
  fg: string | null;
  bg: string | null;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
}

function applyAnsiCodes(codes: number[], state: AnsiState): void {
  let i = 0;
  while (i < codes.length) {
    const c = codes[i];
    if (c === 0) {
      state.fg = null; state.bg = null;
      state.bold = false; state.dim = false; state.italic = false; state.underline = false;
    } else if (c === 1) { state.bold = true; }
    else if (c === 2) { state.dim = true; }
    else if (c === 3) { state.italic = true; }
    else if (c === 4) { state.underline = true; }
    else if (c === 22) { state.bold = false; state.dim = false; }
    else if (c === 23) { state.italic = false; }
    else if (c === 24) { state.underline = false; }
    else if (c === 39) { state.fg = null; }
    else if (c === 49) { state.bg = null; }
    else if (ANSI_FG[c]) { state.fg = ANSI_FG[c]; }
    else if (ANSI_BG[c]) { state.bg = ANSI_BG[c]; }
    else if (c === 38 && codes[i + 1] === 5) {
      state.fg = color256(codes[i + 2]) || state.fg;
      i += 2;
    } else if (c === 48 && codes[i + 1] === 5) {
      state.bg = color256(codes[i + 2]) || state.bg;
      i += 2;
    }
    i++;
  }
}

function stateToStyle(state: AnsiState): string {
  const parts: string[] = [];
  if (state.fg) parts.push(`color:${state.fg}`);
  if (state.bg) parts.push(`background:${state.bg}`);
  if (state.bold) parts.push('font-weight:bold');
  if (state.dim) parts.push('opacity:0.6');
  if (state.italic) parts.push('font-style:italic');
  if (state.underline) parts.push('text-decoration:underline');
  return parts.join(';');
}

/** Render a line with ANSI escape codes into a styled div. */
function renderAnsiLine(text: string, lineType: string): HTMLDivElement {
  const div = document.createElement('div');
  div.className = `terminal-line ${lineType}`;

  // Fast path: no escape codes at all
  if (!text.includes('\x1b')) {
    div.textContent = text;
    return div;
  }

  const state: AnsiState = { fg: null, bg: null, bold: false, dim: false, italic: false, underline: false };
  let lastIndex = 0;

  text.replace(ANSI_SEQ, (match, offset) => {
    // Emit text before this escape sequence
    if (offset > lastIndex) {
      const chunk = text.slice(lastIndex, offset);
      const style = stateToStyle(state);
      if (style) {
        const span = document.createElement('span');
        span.style.cssText = style;
        span.textContent = chunk;
        div.appendChild(span);
      } else {
        div.appendChild(document.createTextNode(chunk));
      }
    }
    lastIndex = offset + match.length;

    // Parse SGR sequence: \x1b[...m
    if (match.startsWith('\x1b[') && match.endsWith('m')) {
      const inner = match.slice(2, -1);
      const codes = inner ? inner.split(';').map(Number) : [0];
      applyAnsiCodes(codes, state);
    }
    // All other sequences (OSC, charset, mode) are silently stripped
    return '';
  });

  // Emit remaining text after last sequence
  if (lastIndex < text.length) {
    const chunk = text.slice(lastIndex);
    const style = stateToStyle(state);
    if (style) {
      const span = document.createElement('span');
      span.style.cssText = style;
      span.textContent = chunk;
      div.appendChild(span);
    } else {
      div.appendChild(document.createTextNode(chunk));
    }
  }

  return div;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface TerminalLine {
  type: 'stdout' | 'stderr' | 'system' | 'success';
  text: string;
}

export interface TerminalProps {
  lines: () => TerminalLine[];
  /** Reactive key — when this changes, terminal does a full rebuild (e.g. process ID). */
  key?: () => string | null;
}

// ---------------------------------------------------------------------------
// Terminal component
// ---------------------------------------------------------------------------

export function Terminal(props: TerminalProps) {
  const el = h('div', { class: 'terminal', style: 'position: relative;' }) as HTMLDivElement;

  // Scroll-to-bottom button — positioned inside the terminal
  const scrollBtn = document.createElement('button');
  scrollBtn.style.cssText = `
    position: sticky; bottom: 8px; left: 100%; transform: translateX(calc(-100% - 12px));
    z-index: 10; background: var(--gruvbox-bg-soft); border: 1px solid var(--gruvbox-border);
    border-radius: 4px; padding: 4px 10px; cursor: pointer;
    font-family: var(--font-code); font-size: 11px; color: var(--gruvbox-gray);
    display: none; box-shadow: 0 2px 8px rgba(0,0,0,0.3);
  `;
  scrollBtn.textContent = '↓ Bottom';
  scrollBtn.onmouseenter = () => { scrollBtn.style.color = 'var(--accent, var(--gruvbox-yellow))'; scrollBtn.style.borderColor = 'var(--accent, var(--gruvbox-yellow))'; };
  scrollBtn.onmouseleave = () => { scrollBtn.style.color = 'var(--gruvbox-gray)'; scrollBtn.style.borderColor = 'var(--gruvbox-border)'; };
  scrollBtn.onclick = (e) => {
    e.stopPropagation();
    el.scrollTop = el.scrollHeight;
    userScrolledUp = false;
    scrollBtn.style.display = 'none';
  };

  el.appendChild(scrollBtn);

  // Track whether user has scrolled up
  let userScrolledUp = false;

  function isNearBottom(): boolean {
    return el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  }

  el.addEventListener('scroll', () => {
    if (isNearBottom()) {
      userScrolledUp = false;
      scrollBtn.style.display = 'none';
    } else {
      userScrolledUp = true;
      scrollBtn.style.display = 'block';
    }
  });

  // Track what we've rendered to enable incremental append
  let renderedCount = 0;
  let renderedIdentity: string | null = null;

  createEffect(() => {
    const allLines = props.lines();

    // Use the key prop if provided (e.g. process ID), otherwise fall back to first line
    const identity = props.key ? props.key() : (allLines.length > 0 ? allLines[0].text : null);

    const needsFullRebuild =
      identity !== renderedIdentity ||
      allLines.length < renderedCount;

    if (needsFullRebuild) {
      el.innerHTML = '';
      el.appendChild(scrollBtn); // keep the button in the DOM
      renderedCount = 0;
      renderedIdentity = identity;
      userScrolledUp = false;
      scrollBtn.style.display = 'none';
    }

    // Append new lines before the scroll button
    for (let i = renderedCount; i < allLines.length; i++) {
      const line = allLines[i];
      const lineEl = renderAnsiLine(line.text, line.type);
      el.insertBefore(lineEl, scrollBtn);
    }

    renderedCount = allLines.length;

    // Auto-scroll only if user hasn't scrolled up
    if (!userScrolledUp) {
      el.scrollTop = el.scrollHeight;
    }
  });

  return el;
}
