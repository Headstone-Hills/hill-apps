import { execSync } from 'node:child_process';
import { defineConfig } from 'vitest/config';

/** "27611ef · 1 Nov 00:38": which commit and when this build was made, shown next to the title so a stale copy is obvious. */
function buildId(): string {
  let hash = 'dev';
  try {
    hash = execSync('git rev-parse --short HEAD', { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
    if (execSync('git status --porcelain', { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim()) hash += '+';
  } catch {
    /* not a git checkout: leave it as "dev" */
  }
  const d = new Date();
  const pad = (n: number): string => String(n).padStart(2, '0');
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return `${hash} · ${d.getDate()} ${months[d.getMonth()]} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export default defineConfig({
  // Relative base so the built app also works from a file-based Electron wrapper later.
  base: './',
  worker: { format: 'es' },
  define: { __BUILD_ID__: JSON.stringify(buildId()) },
  test: { environment: 'node', include: ['tests/**/*.test.ts'] },
});
