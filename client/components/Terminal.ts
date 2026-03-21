import { h, createEffect } from '@getforma/core';

// ---------------------------------------------------------------------------
// ANSI escape code stripping
// ---------------------------------------------------------------------------

const ANSI_REGEX = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07|\x1b[()][AB012]|\x1b[=>]/g;

function stripAnsi(text: string): string {
  return text.replace(ANSI_REGEX, '');
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
      const lineEl = document.createElement('div');
      lineEl.className = `terminal-line ${line.type}`;
      lineEl.textContent = stripAnsi(line.text);
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
