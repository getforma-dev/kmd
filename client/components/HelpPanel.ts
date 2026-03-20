import { h } from '@getforma/core';

/**
 * HelpPanel — overlay showing CLI commands, keyboard shortcuts, and tips.
 */
export function HelpPanel(props: { onClose: () => void }) {
  const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0;
  const mod = isMac ? '⌘' : 'Ctrl+';

  function Section(title: string, ...children: unknown[]) {
    return h('div', { style: 'margin-bottom: 20px;' },
      h('div', {
        style: 'font-family: var(--font-mono, var(--font-code)); font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--accent, var(--gruvbox-yellow)); margin-bottom: 8px;',
      }, title),
      ...children,
    );
  }

  function Row(left: string, right: string) {
    return h('div', {
      style: 'display: flex; justify-content: space-between; align-items: center; padding: 4px 0; font-size: 13px;',
    },
      h('span', { style: 'color: var(--gruvbox-fg);' }, left),
      h('kbd', { class: 'kbd', style: 'font-size: 11px;' }, right),
    );
  }

  function CmdRow(cmd: string, desc: string) {
    return h('div', {
      style: 'display: flex; gap: 12px; padding: 4px 0; font-size: 13px; align-items: baseline;',
    },
      h('code', {
        style: 'font-family: var(--font-mono, var(--font-code)); color: var(--accent, var(--gruvbox-yellow)); font-size: 12px; white-space: nowrap;',
      }, cmd),
      h('span', { style: 'color: var(--gruvbox-gray); font-size: 12px;' }, desc),
    );
  }

  // Backdrop
  const backdrop = h('div', {
    style: 'position: fixed; inset: 0; background: rgba(0,0,0,0.5); z-index: 1000; display: flex; align-items: center; justify-content: center;',
    onClick: (e: Event) => {
      if (e.target === backdrop) props.onClose();
    },
  },
    // Panel
    h('div', {
      style: `
        background: var(--gruvbox-bg-soft);
        border: 1px solid var(--gruvbox-border);
        border-radius: 8px;
        width: 560px;
        max-height: 80vh;
        overflow-y: auto;
        padding: 24px;
      `,
      onClick: (e: Event) => e.stopPropagation(),
    },
      // Header
      h('div', {
        style: 'display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;',
      },
        h('div', null,
          h('span', {
            style: 'font-family: var(--font-mono, var(--font-code)); font-size: 18px; font-weight: 700; color: var(--gruvbox-fg);',
          },
            'K',
            h('span', { style: 'color: var(--accent, var(--gruvbox-yellow));' }, '.'),
            h('span', { style: 'font-weight: 400; color: var(--gruvbox-gray);' }, 'md'),
          ),
          h('span', {
            style: 'font-size: 11px; color: var(--gruvbox-gray); margin-left: 8px;',
          }, 'v0.1.0'),
        ),
        h('button', {
          class: 'btn btn-ghost',
          style: 'padding: 4px 8px; font-size: 12px;',
          onClick: () => props.onClose(),
        }, 'Close'),
      ),

      // Keyboard shortcuts
      Section('Keyboard Shortcuts',
        Row('Command palette', `${mod}K`),
        Row('Docs tab', `${mod}1`),
        Row('Scripts tab', `${mod}2`),
        Row('Ports tab', `${mod}3`),
        Row('Terminal tab', `${mod}4`),
        Row('Close / clear', 'Esc'),
        Row('Help', '?'),
      ),

      // CLI commands
      Section('CLI Commands',
        CmdRow('kmd', 'Start server (ephemeral or workspace)'),
        CmdRow('kmd init', 'Create .kmd/ workspace (opt-in)'),
        CmdRow('kmd init --name "Foo"', 'Init with a custom name'),
        CmdRow('kmd status', 'Check if workspace server is running'),
        CmdRow('kmd add <path>', 'Add root to workspace'),
        CmdRow('kmd remove <path>', 'Remove root from workspace'),
        CmdRow('kmd list', 'Show workspace or ephemeral info'),
        CmdRow('kmd --port 8080', 'Use a custom port'),
        CmdRow('kmd --no-open', 'Start without opening browser'),
        CmdRow('kmd --force', 'Skip project root warnings'),
      ),

      // Tips
      Section('Tips',
        h('div', { style: 'font-size: 12px; color: var(--gruvbox-gray); line-height: 1.7;' },
          h('p', { style: 'margin: 0 0 6px;' },
            h('strong', { style: 'color: var(--gruvbox-fg);' }, 'Two modes: '),
            'Run ',
            h('code', { style: 'font-size: 11px; color: var(--accent, var(--gruvbox-yellow));' }, 'kmd'),
            ' for ephemeral (no files left behind), or ',
            h('code', { style: 'font-size: 11px; color: var(--accent, var(--gruvbox-yellow));' }, 'kmd init'),
            ' for a persistent workspace.',
          ),
          h('p', { style: 'margin: 0 0 6px;' },
            h('strong', { style: 'color: var(--gruvbox-fg);' }, 'Multi-root workspaces: '),
            'Run ',
            h('code', { style: 'font-size: 11px; color: var(--accent, var(--gruvbox-yellow));' }, 'kmd add ../other-project'),
            ' to index docs and scripts from multiple projects in one UI.',
          ),
          h('p', { style: 'margin: 0 0 6px;' },
            h('strong', { style: 'color: var(--gruvbox-fg);' }, 'Ports: '),
            'Workspaces use port 4444 (fixed). Ephemeral sessions auto-pick from 4445-4460.',
          ),
          h('p', { style: 'margin: 0;' },
            h('strong', { style: 'color: var(--gruvbox-fg);' }, 'Fully offline: '),
            'Everything including mermaid diagrams works without internet.',
          ),
        ),
      ),
    ),
  );

  return backdrop;
}
