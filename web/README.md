# `web/` — the Pochoir interface

Astro, static output, one Svelte island, hand-written CSS. No runtime
dependency reaches the browser. The design system this is built to is
[DESIGN.md](DESIGN.md); read it before changing how anything looks.

```
pnpm install
pnpm dev        # http://localhost:4321
pnpm build      # -> dist/, content-addressed filenames
pnpm check      # astro check: types across .astro, .svelte and .ts
```

There is no Makefile target for this yet. `make` is the repository's only
entry point and the root `Makefile` is not this directory's to edit, so adding
`web-build` / `web-check` targets and calling them from CI is a change the
owner needs to make.

## Layout

```
src/
  engine/      the seam between the UI and the Rust/wasm engine
  components/  the Svelte island and its parts
  i18n/        every user-visible string
  layouts/     the page frame
  pages/       index.astro (the tool), confidentialite.astro
  styles/      tokens.css, base.css, app.css
```

## The engine seam

The UI talks to `PochoirModule` (`src/engine/module.ts`) and to nothing else.
Two implementations sit behind it:

| File | What it is |
| --- | --- |
| `abi.ts` | The real thing: a thin wrapper over the WebAssembly ABI below. Written, typed, unused. |
| `module-stub.ts` | A stub that composites with the browser's own canvas, so the interface is fully exercisable today. |
| `select.ts` | **The one file to change** when the engine lands. |

Everything runs inside a Web Worker (`worker.ts`) that the main thread is
willing to `terminate()` — `client.ts` owns the deadline, kills the worker when
a render overruns it, and re-uploads the image to a fresh one on the next call.
That is required by ADR-0003 (`panic = "abort"` traps the instance) and by not
wanting a 12 MP photograph to freeze the page. It is already true against the
stub, so the behaviour is not something the real engine will have to introduce.

### The ABI this is written against

Synchronous, called from a worker, no `wasm-bindgen`, plain exports over linear
memory:

```
pochoir_alloc(len: u32) -> u32
pochoir_free(ptr: u32, len: u32)

pochoir_render_pdf(img_ptr, img_len, text_ptr, text_len, opts_ptr, opts_len) -> u32
pochoir_render_preview(img_ptr, img_len, text_ptr, text_len, opts_ptr, opts_len, max_px) -> u32

pochoir_result_status(h) -> u32   // 0 = success
pochoir_result_ptr(h)    -> u32
pochoir_result_len(h)    -> u32
pochoir_result_width(h)  -> u32
pochoir_result_height(h) -> u32
pochoir_result_free(h)
```

The mark text is a separate UTF-8 buffer. The options are a packed
little-endian struct of fixed fields, 48 bytes, defined once in
`src/engine/options.ts` and reproduced here so the Rust side has something to
diff against:

| Offset | Type | Field |
| --- | --- | --- |
| 0 | `u32` | version (currently 1) |
| 4 | `u32` | quality mode — 0 compact, 1 standard, 2 high, 3 lossless |
| 8 | `u32` | page sizing — 0 `a4-max`, 1 `native` |
| 12 | `u32` | DPI |
| 16 | `f32` | font size, per cent of the page's shorter edge |
| 20 | `f32` | angle, degrees |
| 24 | `f32` | opacity, 0..1 |
| 28 | `f32` | blur, px at render resolution |
| 32 | `u32` | tile density — 0 low, 1 normal, 2 high |
| 36 | `u32` | colour, `0x00RRGGBB` |
| 40 | `u64` | jitter seed |

`pochoir_render_preview` is expected to return **RGBA8, row-major, no padding**,
with `width`/`height` set. `pochoir_render_pdf` returns the PDF bytes with
`width`/`height` zero. The wrapper copies results out of linear memory before
making any further call, because growing the memory detaches every view over it.

## The CSP this page needs

`web/` does not own the response headers — the deploy does. This is the exact
policy the build is written to, and it has been **tested**: the `dist/` output
was served behind precisely this header and the page mounts, the worker starts,
a preview renders and a PDF downloads, with no violation reported.

```
Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; manifest-src 'none'; media-src 'none'
```

As a Cloudflare `_headers` rule:

```
/*
  Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; manifest-src 'none'; media-src 'none'
  Cross-Origin-Opener-Policy: same-origin
  X-Content-Type-Options: nosniff
  Referrer-Policy: no-referrer

/_pochoir/*
  Cache-Control: public, max-age=31536000, immutable
```

Notes, in the order they will matter:

- **`'wasm-unsafe-eval'` is required** and `'unsafe-eval'` is deliberately not
  paired with it (ADR-0004 Decision 5). Instantiation fails with a
  `WebAssembly.CompileError` without it, and it fails at instantiation rather
  than at fetch, so it is easy to miss until the module does not load.
- **No `'unsafe-inline'` is needed, anywhere, and the build is arranged to keep
  it that way.** `build.inlineStylesheets: 'never'` stops Astro inlining small
  stylesheets, `vite.build.assetsInlineLimit: 0` stops assets becoming `data:`
  URIs, and the single island is mounted by hand in `src/pages/index.astro`
  rather than with a `client:*` directive — Astro's island machinery emits
  three inline things (the `astro-island` element, its bootstrap, and a
  `display: contents` style block) and every one of them would force
  `'unsafe-inline'`. If someone adds a `client:load` later, the policy breaks
  and the page goes blank. The built HTML currently contains **zero** inline
  `<script>`, zero inline `<style>` and zero `style=` attributes; that is worth
  asserting in CI.
- **`worker-src 'self'`, not `blob:`.** Vite emits the worker as a real
  same-origin file, confirmed in the network log.
- **`connect-src 'self'` is the line a reviewer is meant to read**, and it is
  the one that makes the product claim checkable. Note what it is not: CSP has
  no shipped directive restricting top-level navigation, so it is strong
  evidence about intent rather than a seal on the origin. The confidentiality
  page says exactly that and no more.
- **The PDF download is not covered by `connect-src`.** It is an
  `<a download href="blob:">`, which is a download rather than a fetch, and it
  was verified to work under the policy above. A *fetch* of a `blob:` URL is
  blocked — which is correct, and which the app never does.
- `script-src` has no `'self' blob:`; nothing creates a worker or a script from
  a blob.

## Content-addressed filenames

Every built asset carries a content hash (`_pochoir/[name].[hash][extname]`).
Per ADR-0004 this is a correctness requirement, not caching: Cloudflare can
serve HTML and assets from different versions mid-rollout, and SRI fails
closed when they disagree. Do not switch it off for tidier URLs.

## Dependencies

Runtime, shipped to the browser: **none.** The CSS is hand-written with custom
properties, the icons are inline SVG paths, the drop target is `dragover` /
`drop` by hand, and Svelte 5 runes are the state management — which is what
ADR-0001's four `None` rows already decided.

Build-time, all `devDependencies`:

| Package | Why |
| --- | --- |
| `astro` | The static site generator ADR-0001 already chose. |
| `@astrojs/svelte` | Teaches Vite to compile `.svelte`. ADR-0001 chose Svelte islands; this is how they build. |
| `svelte` | The component framework, compiled away at build time apart from its small runtime. |
| `typescript` | Pinned to `^5`: `@astrojs/check` and `svelte2tsx` both refuse TypeScript 7, which pnpm resolves to by default. |
| `@astrojs/check` | `pnpm check`. Type-checks `.astro` files, which `tsc` alone will not. |

No ADR was needed: nothing here departs from ADR-0001's rows. That was the
goal.

## Strings and language

French only. Every user-visible string is in `src/i18n/` — `fr.ts` for the
interface, `fr-privacy.ts` for the long-form page. Components import `t` and
hold no literals. Adding English is: copy both files, translate, add the locale
to the map in `src/i18n/index.ts`, add a switch. It is an addition, not a
rewrite, which was the requirement.

## Accessibility

RGAA AA / WCAG AA is a stated requirement here, not a polish pass, because we
are competing with a government tool that has declared itself non-conformant
since September 2023. What was done and how it was checked is in the pull
request description; the short version:

- One focus treatment, `:focus-visible`, 2px ink with a 2px offset, nowhere
  suppressed. The file input is a real focusable `<input type="file">` that is
  visually hidden, with its wrapper showing the ring via `:focus-within`.
- Measured contrast for every token pair that is actually used, in DESIGN.md,
  including the one combination that fails and is therefore forbidden.
- The preview announces itself through a polite live region when a render
  settles — not on every intermediate frame while a slider is being dragged.
- Reflow verified with no horizontal scrolling at 720 CSS px (200% zoom of a
  1440px viewport) and at 360 px.
- Status is never carried by colour alone.

## Not done here, on purpose

No deployment, no Cloudflare config, no CI wiring, no PDF input, no HEIC, no
multi-file handling, no exclusion zone, no PWA, no dark mode.
