import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [sveltekit({
    adapter: adapter({ fallback: 'index.html' }),
    paths: { relative: false }
  })],
  resolve: {
    alias: { '@icons': fileURLToPath(new URL('../../fiend-desktop/assets/icons', import.meta.url)) }
  }
});
