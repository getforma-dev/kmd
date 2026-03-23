import { h, createSignal, createEffect, onCleanup } from '@getforma/core';
import { iconDocs, iconScripts, iconPorts, iconTerminal } from './icons';

export type Route = 'docs' | 'scripts' | 'ports' | 'terminal';

// ---------------------------------------------------------------------------
// Feature 4: Git status polling
// ---------------------------------------------------------------------------

interface GitRootStatus {
  root: string;
  branch: string | null;
  dirty_count: number;
  is_dirty: boolean;
  head_short: string | null;
}

function useGitStatus() {
  const [gitStatus, setGitStatus] = createSignal<GitRootStatus[]>([]);

  function refresh() {
    fetch('/api/git/status')
      .then((r) => r.json())
      .then((data: { roots: GitRootStatus[] }) => setGitStatus(data.roots || []))
      .catch(() => {});
  }

  refresh();
  const timer = setInterval(refresh, 15000); // Poll every 15s
  onCleanup(() => clearInterval(timer));

  return gitStatus;
}

const LABELS: Record<Route, string> = {
  docs: 'Docs',
  scripts: 'Scripts',
  ports: 'Ports',
  terminal: 'Terminal',
};

const ICON_FNS: Record<Route, () => SVGElement> = {
  docs: iconDocs,
  scripts: iconScripts,
  ports: iconPorts,
  terminal: iconTerminal,
};

export function Sidebar(props: {
  route: () => Route;
  workspaceName?: () => string;
  theme?: () => string;
  crashCount?: () => number;
  onToggleTheme?: () => void;
  onHelp?: () => void;
  onWorkspaceSettings?: () => void;
}) {
  const navItems = (['docs', 'scripts', 'ports', 'terminal'] as const).map((key) =>
    h('a', {
      class: () => `nav-item${props.route() === key ? ' active' : ''}`,
      href: `#${key}`,
      style: 'position: relative;',
    },
      ICON_FNS[key](),
      h('span', null, LABELS[key]),
      // Crash badge on Scripts tab
      key === 'scripts' && props.crashCount
        ? h('span', {
            class: 'crash-badge',
            style: () => {
              const count = props.crashCount!();
              return count > 0
                ? 'position: absolute; top: 4px; right: 8px; background: #fb4934; color: #1d2021; font-size: 9px; font-weight: 700; min-width: 14px; height: 14px; border-radius: 7px; display: flex; align-items: center; justify-content: center; font-family: var(--font-code);'
                : 'display: none;';
            },
          }, () => {
            const count = props.crashCount!();
            return count > 0 ? String(count) : '';
          })
        : null,
    )
  );

  // Sun icon for light mode, moon icon for dark mode
  function ThemeToggle() {
    if (!props.onToggleTheme) return h('div', null);

    return h('button', {
      class: 'theme-toggle-btn',
      onClick: () => props.onToggleTheme!(),
      title: () => props.theme?.() === 'light' ? 'Switch to dark mode' : 'Switch to light mode',
    },
      // Sun icon (shown when in dark mode — click to go light)
      h('svg', {
        viewBox: '0 0 24 24',
        fill: 'none',
        stroke: 'currentColor',
        'stroke-width': '1.5',
        'stroke-linecap': 'round',
        'stroke-linejoin': 'round',
        style: () => `width: 16px; height: 16px; ${props.theme?.() === 'light' ? 'display: none;' : ''}`,
      },
        h('circle', { cx: '12', cy: '12', r: '5' }),
        h('line', { x1: '12', y1: '1', x2: '12', y2: '3' }),
        h('line', { x1: '12', y1: '21', x2: '12', y2: '23' }),
        h('line', { x1: '4.22', y1: '4.22', x2: '5.64', y2: '5.64' }),
        h('line', { x1: '18.36', y1: '18.36', x2: '19.78', y2: '19.78' }),
        h('line', { x1: '1', y1: '12', x2: '3', y2: '12' }),
        h('line', { x1: '21', y1: '12', x2: '23', y2: '12' }),
        h('line', { x1: '4.22', y1: '19.78', x2: '5.64', y2: '18.36' }),
        h('line', { x1: '18.36', y1: '5.64', x2: '19.78', y2: '4.22' }),
      ),
      // Moon icon (shown when in light mode — click to go dark)
      h('svg', {
        viewBox: '0 0 24 24',
        fill: 'none',
        stroke: 'currentColor',
        'stroke-width': '1.5',
        'stroke-linecap': 'round',
        'stroke-linejoin': 'round',
        style: () => `width: 16px; height: 16px; ${props.theme?.() === 'light' ? '' : 'display: none;'}`,
      },
        h('path', { d: 'M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z' }),
      ),
    );
  }

  // Feature 4: Git status
  const gitStatus = useGitStatus();

  function GitStatusIndicator() {
    const container = document.createElement('div');
    container.style.cssText = 'padding: 0 16px 8px; font-size: 11px; font-family: var(--font-code);';

    createEffect(() => {
      const statuses = gitStatus();
      container.innerHTML = '';
      if (statuses.length === 0) return;

      for (const s of statuses) {
        if (!s.branch) continue;
        const row = document.createElement('div');
        row.style.cssText = 'display: flex; align-items: center; gap: 5px; padding: 2px 0; color: var(--gruvbox-fg2);';

        // Branch icon (git branch)
        const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        icon.setAttribute('viewBox', '0 0 24 24');
        icon.setAttribute('fill', 'none');
        icon.setAttribute('stroke', 'currentColor');
        icon.setAttribute('stroke-width', '2');
        icon.setAttribute('stroke-linecap', 'round');
        icon.style.cssText = 'width: 12px; height: 12px; flex-shrink: 0; opacity: 0.6;';
        const line1 = document.createElementNS('http://www.w3.org/2000/svg', 'line');
        line1.setAttribute('x1', '6'); line1.setAttribute('y1', '3');
        line1.setAttribute('x2', '6'); line1.setAttribute('y2', '15');
        const circle1 = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
        circle1.setAttribute('cx', '18'); circle1.setAttribute('cy', '6'); circle1.setAttribute('r', '3');
        const circle2 = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
        circle2.setAttribute('cx', '6'); circle2.setAttribute('cy', '18'); circle2.setAttribute('r', '3');
        const path1 = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path1.setAttribute('d', 'M18 9a9 9 0 0 1-9 9');
        icon.appendChild(line1);
        icon.appendChild(circle1);
        icon.appendChild(circle2);
        icon.appendChild(path1);
        row.appendChild(icon);

        // Branch name
        const branch = document.createElement('span');
        branch.style.cssText = 'overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 120px;';
        branch.textContent = s.branch;
        branch.title = s.branch;
        row.appendChild(branch);

        // Dirty indicator
        if (s.is_dirty) {
          const dirty = document.createElement('span');
          dirty.style.cssText = 'color: var(--gruvbox-yellow); font-size: 10px;';
          dirty.textContent = `+${s.dirty_count}`;
          dirty.title = `${s.dirty_count} uncommitted change${s.dirty_count === 1 ? '' : 's'}`;
          row.appendChild(dirty);
        }

        // Short hash
        if (s.head_short) {
          const hash = document.createElement('span');
          hash.style.cssText = 'color: var(--gruvbox-gray); font-size: 10px; margin-left: auto;';
          hash.textContent = s.head_short;
          row.appendChild(hash);
        }

        container.appendChild(row);
      }
    });

    return container;
  }

  return h('aside', { class: 'sidebar' },
    h('div', { class: 'sidebar-header' },
      h('span', { class: 'sidebar-logo' },
        'K',
        h('span', { class: 'dot' }, '.'),
        h('span', { class: 'md' }, 'md'),
      ),
      props.workspaceName
        ? h('span', {
            class: 'sidebar-project-name',
            title: () => props.workspaceName!(),
          }, () => props.workspaceName!())
        : h('span', { class: 'sidebar-version' }, 'v0.1.0'),
    ),
    GitStatusIndicator(),
    h('nav', { class: 'sidebar-nav' }, ...navItems),
    h('div', { class: 'sidebar-footer' },
      h('button', {
        class: 'theme-toggle-btn',
        onClick: () => props.onHelp?.(),
        title: 'Help & shortcuts (?)',
        style: 'font-size: 12px;',
      }, '?'),
      h('button', {
        class: 'theme-toggle-btn',
        onClick: () => props.onWorkspaceSettings?.(),
        title: 'Workspace settings',
      },
        h('svg', {
          viewBox: '0 0 24 24',
          fill: 'none',
          stroke: 'currentColor',
          'stroke-width': '1.5',
          'stroke-linecap': 'round',
          'stroke-linejoin': 'round',
          style: 'width: 16px; height: 16px;',
        },
          h('circle', { cx: '12', cy: '12', r: '3' }),
          h('path', { d: 'M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z' }),
        ),
      ),
      ThemeToggle(),
    ),
  );
}
