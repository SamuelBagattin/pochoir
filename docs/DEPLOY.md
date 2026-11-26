# Deploying Pochoir

**Nothing is deployed. There is no Cloudflare account, no zone, no domain, and
no credential anywhere in this repository.** `pochoir.app` is not registered.
This document exists so that the first deploy is a reviewed command rather than
an improvised one.

The one command, once the account exists:

```
make web-deploy
```

It refuses immediately if `CLOUDFLARE_API_TOKEN` is unset, and it depends on
`web-check`, so it cannot publish a bundle that has not been built, attested
and asserted. In CI the same thing runs as the `Deploy` workflow, which is
`workflow_dispatch`-only and requires typing `deploy` into a confirmation
input.

## What a deploy does

| Step | Target | Why |
| --- | --- | --- |
| Install from the committed lock file | `web-install` | `--frozen-lockfile`; a build that resolves something else is not the build CI ran |
| Build the site | `web-build` | Astro, static output, content-addressed filenames |
| Attest it | `web-build` | SHA-384 into every `integrity` attribute, a `modulepreload` link per imported chunk, and `manifest.json` |
| Type-check and assert | `web-check` | `astro check`, no inline script or style, every hash matches its file |
| Build twice and compare | `web-reproduce` | ADR-0004 Decision 4 items 1 and 2: a release that cannot reproduce itself does not ship |
| Resolve the headers | `web-deploy-check` | prints what `_headers` yields for the root, an asset and the engine |
| Dry run | `web-deploy-check` | `wrangler deploy --dry-run`; validates `wrangler.jsonc` and needs no credential |
| Upload | `web-deploy` | `wrangler deploy` — the only step that needs a token |
| Verify what is served | `web-verify-live` | fetches every path from the origin and compares it with the manifest |

Everything except the last two runs today, offline, on a laptop. Run
`make web-deploy-check` to see it.

## Looking at the real thing locally

`astro preview` sends none of the production response headers, so the strictest
part of this site is invisible in it. Use:

```
make web-serve
```

That serves `web/dist` with the `_headers` rules applied — the same file that
ships inside the bundle — including the Content-Security-Policy and
`Integrity-Policy`. A browser pointed at it enforces the real policy. It
reproduces Cloudflare's rule stacking and comma-joining on purpose, so a
mistake that would break the policy in production breaks it here too.

To see what a path's headers resolve to without starting anything:

```
./scripts/web-serve.py --dist web/dist --explain /_pochoir/engine/x.wasm
```

## What is missing before a live site

In the order someone would have to do them.

1. **A domain.** `pochoir.app` is not registered. Not bought, deliberately.
2. **A Cloudflare account**, and the zone for whatever domain is chosen.
3. **`CLOUDFLARE_ACCOUNT_ID`** as a repository secret. It is not in
   `wrangler.jsonc` on purpose; the repository is public.
4. **`CLOUDFLARE_API_TOKEN`** as a repository secret. Scope it to the minimum:
   *Account → Workers Scripts → Edit* for the one account, and nothing else. Not
   a Global API Key. The token is the thing that can replace the bytes a visitor
   executes, which is the asset this whole project is about protecting.
5. **A `production` GitHub environment** with a required reviewer, so that
   publishing needs a second pair of eyes. The workflow already names it.
6. **`POCHOIR_ORIGIN`** as a repository variable, for the post-deploy check.
7. **A first deploy checked in all three engines.** `'wasm-unsafe-eval'` fails
   at *instantiation*, not at fetch, so a missing keyword looks like a working
   page until someone chooses a file.
8. **A scheduled `web-verify-live`.** ADR-0004 Decision 4 item 6 is explicit
   that a release-time check cannot see a bundle swapped a week later. One
   scheduled workflow, one `make web-verify-live ORIGIN=…`.
9. **Cloudflare's versions API** in that scheduled job, asserting the live
   version's `metadata.source` is `workersci` rather than `dash` or
   `wrangler` — the tripwire for a deploy from a laptop. Retention is 30 days
   except the 100 most recent versions, so it is a tripwire and not an archive.

## What could not be verified without an account

Stated as unknowns rather than guessed at.

- **Whether Cloudflare accepts `#` comments in `_headers` the way this file
  uses them.** The format documents comments; nothing documents an indented
  one. Every comment here therefore sits at column 0 and never inside a rule
  body. If a deploy shows a mangled header, this is the first thing to check.
- **Whether `! Header-Name` detaches as documented.** The local server
  implements the documented behaviour and the rules resolve correctly there.
  Unverified against the platform.
- **Whether `Integrity-Policy`, `Cross-Origin-Opener-Policy` and
  `Content-Type` survive `_headers` at all.** ADR-0004 records the COOP/COEP
  question as open, and no Cloudflare page mentions either header. One
  `curl -I` against the first preview settles all of it.
- **Whether `/_pochoir/engine/*` matches as expected.** Only the trailing
  splat is documented, which is why the engine has its own directory instead of
  a `*.wasm` pattern.
- **The 404 path.** See below; it follows from Cloudflare's documented rule,
  not from a measurement.
- **Cross-host reproducibility.** `make web-reproduce` compares two builds on
  one machine. Whether a Linux CI runner and a macOS laptop produce the same
  `files_digest` is unmeasured for the web bundle, as ADR-0004 records it is
  for the wasm one.

## A finding worth knowing before the first deploy

**An assets-only deployment is not literally Worker-free.** `wrangler deploy
--dry-run` writes a `no-op-worker.js` whose `fetch()` returns
`new Response("Not found", { status: 404, headers: { "Content-Type":
"text/html" }})`, and uploads it. Cloudflare documents that `_headers` is not
applied to responses generated by Worker code. So the 404 response on this site
carries no CSP, no `nosniff` and no `Referrer-Policy`, and is labelled
`text/html`.

It serves a fixed eleven-byte string, so this is a header gap rather than a
risk. But ADR-0001's "no Worker code" premise is narrower than it reads, and
the honest fix is a `404.astro` in the build plus `"not_found_handling":
"404-page"`, which keeps the response inside the asset router where `_headers`
applies. That needs a page written in French and is left to whoever owns
`web/src`.

## The engine

It is wired. `select.ts` returns the real `WasmModule`, nothing imports the
stub any more, and a bundle without an engine therefore renders nothing at
all. So the engine is part of the deployable site, not an extra:

```
make web-site
```

That is `make wasm`, then `make web-engine ENGINE=…`, then `make web-check`,
then one more assertion. `web-deploy` and `web-deploy-check` both depend on
it, so there is no way to publish a bundle whose engine is missing or
unhashed.

`web-engine` places the module at
`web/public/_pochoir/engine/pochoir_wasm.<hash>.wasm` — before the build, so
Astro copies it and the hash ends up inside the bundle it describes — and
rewrites **two lines** of `web/src/engine/engine-manifest.ts`, whose doc
comment is the contract:

```ts
export const ENGINE_URL: string = '/_pochoir/engine/pochoir_wasm.<hash>.wasm';
export const ENGINE_INTEGRITY: string | undefined = 'sha384-…';
```

Only the two `export const` values are touched, so the contract comment
travels with the file and the working diff while an engine is placed is
exactly two lines.

**Getting back to `pnpm dev`.** `make web-engine ENGINE=--clear` removes the
placed module and puts both fallback values back; the result is byte-identical
to the committed file, so `git diff` is empty. Then drop the artefact at
`web/public/pochoir_wasm.wasm` by hand — uncommitted, unhashed, no integrity —
which is what the committed fallback points at.

The module lives in its own `_pochoir/engine/` directory rather than beside the
other assets for one reason: `_headers` has to give it
`Content-Type: application/wasm`, and a trailing splat is the only pattern form
Cloudflare documents. The contract's example path shows
`/_pochoir/pochoir_wasm.<hash>.wasm`; the three numbered requirements are all
met either way, and the subdirectory is what makes requirement 3 expressible.

**Two guards against shipping the development fallback.**
`web-attest.py --verify --require-engine`, which only the deploy path runs,
fails if there is no hashed module in `_pochoir/engine/`, if any `.wasm` sits
outside it, or if any built JavaScript still contains the string
`/pochoir_wasm.wasm` — the exact tell that the generator never ran. Both were
negative-tested.

What remains unimplemented from ADR-0004 Decision 4 after that: the
`rustflags` field in the manifest's `build` section, which should record what
`make wasm` actually used; and publishing the manifest as a GitHub release
asset (item 5), which is the half of Decision 3 that makes the record evidence
rather than convenience.

The *wasm* artefact's own double build is **not** missing — the bindings
session landed `scripts/check-repro.sh` and `make wasm-repro` for it. The
deploy workflow should call that alongside `make web-reproduce` once the two
branches are merged; it does not yet, because `wasm-repro` did not exist when
this workflow was written.
