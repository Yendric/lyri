import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [svelte()],
  server: {
    // Point the dev server at a running Lyri host for live state.
    proxy: {
      '/ws': { target: 'ws://localhost:8080', ws: true }
    }
  }
});
