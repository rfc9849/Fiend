import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const icons = fileURLToPath(new URL('../../fiend-desktop/assets/icons', import.meta.url));
if (!existsSync(icons)) throw new Error(`The icon folder is missing: ${icons}`);

export default defineConfig({
  plugins: [sveltekit({
    adapter: adapter({ fallback: 'index.html' }),
    paths: { relative: false }
  })],
  resolve: {
    alias: { '@icons': icons }
  }
});
