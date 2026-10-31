import { defineConfig } from 'vitest/config';

export default defineConfig({
  // Relative base so the built app also works from a file-based Electron wrapper later.
  base: './',
  worker: { format: 'es' },
  test: { environment: 'node', include: ['tests/**/*.test.ts'] },
});
