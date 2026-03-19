import { h, createEffect } from '@getforma/core';

// ---------------------------------------------------------------------------
// ANSI escape code stripping
// ---------------------------------------------------------------------------

// Matches all ANSI escape sequences: CSI (ESC[...m), OSC, and other control sequences
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
}

// ---------------------------------------------------------------------------
// Terminal component
// ---------------------------------------------------------------------------

export function Terminal(props: TerminalProps) {
  // Build the terminal container
  const el = h('div', { class: 'terminal' }) as HTMLDivElement;

  // Track last rendered line count to append only new lines
  let renderedCount = 0;

  createEffect(() => {
    const allLines = props.lines();

    // Lines were cleared or reset — full rebuild
    if (allLines.length < renderedCount) {
      el.innerHTML = '';
      renderedCount = 0;
    }

    // Append only new lines
    for (let i = renderedCount; i < allLines.length; i++) {
      const line = allLines[i];
      const lineEl = document.createElement('div');
      lineEl.className = `terminal-line ${line.type}`;
      lineEl.textContent = stripAnsi(line.text);
      el.appendChild(lineEl);
    }

    renderedCount = allLines.length;

    // Auto-scroll to bottom
    el.scrollTop = el.scrollHeight;
  });

  return el;
}
