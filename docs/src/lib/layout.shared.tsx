import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { appName, gitConfig } from './shared';

// The wordmark's mark: a 3×3 grid of 4px squares on a 16px viewBox with 2px
// gutters — the dot grid is the icon system (DESIGN.md, Shapes).
function DotMark() {
  const cells = [1, 7, 13].flatMap((y) => [1, 7, 13].map((x) => [x, y]));
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" fill="currentColor">
      {cells.map(([x, y]) => (
        <rect key={`${x}-${y}`} x={x - 1} y={y - 1} width="4" height="4" />
      ))}
    </svg>
  );
}

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: (
        <span
          className="inline-flex items-center gap-2 whitespace-nowrap text-[16px] font-bold uppercase tracking-[0.04em]"
          style={{ fontFamily: 'var(--face-display)', color: 'var(--ink)' }}
        >
          <DotMark />
          {appName}
        </span>
      ),
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
    themeSwitch: { mode: 'light-dark-system' },
  };
}
