/**
 * Where the compiled engine lives, and what its bytes must hash to.
 *
 * ## This file is a build output with a committed fallback
 *
 * What is committed here is the **development fallback**: a plain, unhashed
 * path and no integrity, which is what `pnpm dev` and a local `pnpm preview`
 * want, with the artefact dropped into `web/public/` by hand and left
 * uncommitted.
 *
 * A production build must **overwrite this file** before `astro build` runs,
 * emitting exactly the same two exports with real values:
 *
 * ```ts
 * export const ENGINE_URL = '/_pochoir/pochoir_wasm.<content-hash>.wasm';
 * export const ENGINE_INTEGRITY = 'sha384-<base64>';
 * ```
 *
 * Nothing else in `web/src` reads the engine's location, so that is the whole
 * contract. Three things the generator has to get right:
 *
 * 1. **The filename carries a content hash.** ADR-0004 requires it: Cloudflare
 *    can serve HTML and assets from different versions mid-rollout, and
 *    integrity fails closed when they disagree. A stable `/pochoir_wasm.wasm`
 *    would make the one file we most want pinned the one file that breaks.
 * 2. **The hash is computed from the bytes that are actually served**, after
 *    any post-processing, and never copied by hand. ADR-0001 has already been
 *    wrong once from a hand-copied figure.
 * 3. **The file must be served as `Content-Type: application/wasm`.**
 *    `WebAssembly.instantiateStreaming` rejects with a `TypeError` on anything
 *    else, so this is worth one `curl -I` on the first deploy.
 *
 * Generating it as TypeScript rather than reading a JSON manifest at runtime
 * is deliberate: the values end up inside the hashed bundle, so they cannot
 * drift from the HTML that references them, and a missing manifest is a build
 * error rather than a blank page.
 */

/** The URL the engine is fetched from. */
export const ENGINE_URL: string = '/pochoir_wasm.wasm';

/**
 * The SRI string for those bytes, or `undefined` in development.
 *
 * When it is set, `fetch` verifies it and fails closed on a mismatch — the
 * point of ADR-0004 Decision 2. When it is not, the fetch is plain, which is
 * correct for a local file the developer put there themselves and is why the
 * fallback leaves it undefined rather than inventing a value.
 */
export const ENGINE_INTEGRITY: string | undefined = undefined;
