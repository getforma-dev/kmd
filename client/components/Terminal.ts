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
}

// ---------------------------------------------------------------------------
// Terminal component
// ---------------------------------------------------------------------------

export function Terminal(props: TerminalProps) {
  const el = h('div', { class: 'terminal' }) as HTMLDivElement;

  // Track what we've rendered to enable incremental append
  let renderedCount = 0;
  let renderedIdentity: string | null = null; // identity of the current content source

  createEffect(() => {
    const allLines = props.lines();

    // Determine identity from the first line — if it changes, this is a different
    // process/source and we need a full rebuild (not an incremental append)
    const identity = allLines.length > 0 ? allLines[0].text : null;

    const needsFullRebuild =
      identity !== renderedIdentity || // different process/source
      allLines.length < renderedCount;  // lines were cleared

    if (needsFullRebuild) {
      el.innerHTML = '';
      renderedCount = 0;
      renderedIdentity = identity;
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
