// @ts-check
import { defineConfig } from 'astro/config';
import svelte from '@astrojs/svelte';

// Pochoir ships as static files on Cloudflare Workers static assets. There is
// no server, by design: see docs/adr/0001-stack-and-dependency-policy.md.
//
// NOTHING IN THIS BUILD MAY REACH A THIRD PARTY. No font CDN, no analytics, no
// error reporting, no remote image service. A privacy tool that phones home on
// load has refuted itself before the user has chosen a file. If you are about
// to add an integration here, that is the bar it has to clear.
export default defineConfig({
  output: 'static',
  compressHTML: true,
  devToolbar: { enabled: false },

  integrations: [svelte()],

  build: {
    // Content-addressed filenames are a requirement of ADR-0004, not a caching
    // nicety: Cloudflare can serve HTML and assets from different versions
    // mid-rollout, and Subresource Integrity fails closed when they disagree.
    assets: '_pochoir',
    // Astro inlines small stylesheets by default. An inline <style> would
    // force `style-src 'unsafe-inline'` into the CSP, which ADR-0004 forbids.
    inlineStylesheets: 'never',
  },

  vite: {
    build: {
      // Same reasoning: an inlined data: URI asset would force `img-src data:`
      // and would not be covered by a filename hash.
      assetsInlineLimit: 0,
      rollupOptions: {
        output: {
          entryFileNames: '_pochoir/[name].[hash].js',
          chunkFileNames: '_pochoir/[name].[hash].js',
          assetFileNames: '_pochoir/[name].[hash][extname]',
        },
      },
    },
    worker: { format: 'es' },
  },
});
