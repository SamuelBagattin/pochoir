# Deploying Pochoir

> **Superseded in part, 2026-11-27.** The site is live at
> <https://pochoir.sam-bagattin.workers.dev>. The paragraph below described the
> state before that and is kept because most of this document — the steps, the
> order, the list of what a domain still needs — is unchanged. Where a line
> says something is unverified for want of an account, check the *What could
> not be verified* section first: several of those questions are now answered.

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
| Verify what is served | `web-verify-live` | fetches every path from the origin and compares it with the manifest, retrying the whole comparison while the origin is still propagating |

Everything except the last two runs today, offline, on a laptop. Run
`make web-deploy-check` to see it.

## Deployment propagation, and what the live check can prove

**The post-deploy check races the platform, and on 2026-11-27 it lost.** Run
45985959359 uploaded a correct bundle, and the verify step then reported two
content-addressed chunks as `404` and `index.html` as a mismatch, and failed the
job. A hand check about a minute later found the origin entirely correct: both
files 200, the served HTML referencing the new hashes. Nothing was wrong with
the deploy.

Cloudflare documents the mechanism, and it accounts for every symptom
([version skew and static assets](https://developers.cloudflare.com/workers/versions-and-deployments/version-skew/)):
a Worker version owns an **asset manifest** mapping paths to files; "deployment
changes take up to a few seconds to reach every location on Cloudflare's
network", and until they have, "locations can use different deployment
configurations". A request that lands on a location still serving the previous
version asks for a path that version has never heard of, and a missing file can
return a 404. The `index.html` mismatch is the same skew the other way round:
that location served the old HTML. The oddest line of that run — the engine
`.wasm` fetched twice, 1.6 seconds apart, coming back 200-and-correct then 404
— is two requests reaching two locations with two configurations.

So `verify-live.py` now **retries the whole manifest** until one pass is
coherent or a deadline passes. A partial match is not success: the claim worth
making is that every path matched *at the same moment*, so each retry restarts
the comparison instead of patching up the paths that failed last time. Backoff
is 2s, 4s, 8s, then 15s, clamped so the last pass does not overrun.

It separates "not there yet" from "wrong", which is the part a human reading a
red build benefits from:

| Printed | What it is | Behaviour |
| --- | --- | --- |
| `PENDING` | a non-200 on a manifest path, a missing header, an odd 404 probe | retried; at the deadline the run is **INCONCLUSIVE** (exit 3) |
| `STALE` | a 200 whose bytes differ at a path with no content hash (`index.html`) | retried; at the deadline it is a **failure** (exit 1) — two minutes on, this is a deploy that did not take, not a race |
| `MISMATCH` | a 200 whose bytes differ at a **content-addressed** path | **fails on the first pass**, without waiting |

That last row is the one that keeps the gate sharp. Cloudflare's own advice is
content-hashed filenames precisely so that "different file contents then have
different paths" — version skew makes such a path *absent*, never *different*.
Different bytes there are bytes we never built, and no amount of waiting makes
them legitimate. A one-byte flip in a chunk fails in under a second.

**The deadline is 120 seconds** (`DEADLINE=` on the target, `--deadline` on the
script). Where that number comes from, since one chosen to make a single run
pass would be worthless:

- Cloudflare documents propagation as "up to a few seconds". 120s is twenty to
  forty times that, so exhausting it means we are not in the documented
  behaviour any more — which is what this check exists to notice.
- It covers the whole interval the observed race leaves open, with margin: the
  check failed 1.7s in and a hand check about a minute later was clean, so that
  day's convergence lay somewhere in (1.7s, ~60s]. One observation, stated as
  one.
- It is short enough that a genuinely broken deploy is red within two minutes
  of the upload. A gate nobody waits for is a gate nobody runs.
- It is deliberately **not** sized for the reported cases of a Workers deploy
  serving a stale asset manifest for far longer
  ([workers-sdk#16658](https://github.com/cloudflare/workers-sdk/issues/16658)).
  Those are unverified user reports; if we are in that state, red is correct and
  waiting ten minutes to say so would only hide it.

**What a green run proves, stated narrowly**, because ADR-0004 is careful about
this kind of overstatement:

> every path in the manifest was served, with matching bytes, to this client,
> from whichever Cloudflare location this client reached, within N seconds of
> the upload.

Not "the deploy is verified", and not "every location serves this bundle". The
same Cloudflare page bounds what any check of this shape can do, in a sentence
worth keeping: **"Waiting a fixed time or testing one location does not prove
that every location has the change."** The retry buys freedom from a race we
were losing; it buys no knowledge about the other three hundred cities, and the
script's output does not claim any.

**The step still fails the job**, and should. The false red was fixed at its
cause rather than by turning the alarm down: this is the only step in the whole
scheme that sees what a visitor receives, and a warning-only version of it is a
log line nobody reads. A red build here means *we could not confirm what is
being served*, which is a true and recoverable statement — re-run the step. A
green build that said "verified" without having checked would not be
recoverable, because nobody looks twice at green.

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
   scheduled workflow, one `make web-verify-live ORIGIN=… DEADLINE=0`. Pass
   `DEADLINE=0` there: no deploy is in flight, so there is nothing to wait for,
   and a scheduled check that retried for two minutes would be giving a real
   substitution two minutes of grace. See *Deployment propagation* below.
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
- ~~**Whether `Integrity-Policy`, `Cross-Origin-Opener-Policy` and
  `Content-Type` survive `_headers` at all.**~~ **Answered 2026-11-27: they
  do.** `curl -I` against the live origin returns `integrity-policy`,
  `cross-origin-opener-policy`, `referrer-policy`, `x-content-type-options` and
  the Content-Security-Policy intact on one line. The comments parse as
  written and nothing is mangled.

  This was the good news and it came with bad. The enforcing form,
  `Integrity-Policy: blocked-destinations=(script)`, arrived at the browser and
  was obeyed — and **made the site a blank page in Safari**, because WebKit
  does not accept a `modulepreload` link's `integrity` as metadata for the
  import it covers, so every code-split chunk is refused. The header now ships
  as `Integrity-Policy-Report-Only`. The measurements are in
  [ADR-0009](adr/0009-deploy-plumbing-and-web-attestation.md) Decision 5 and the
  consequence for what we can claim is in
  [ADR-0004](adr/0004-build-reproducibility-and-deployed-bundle-attestation.md),
  the ceiling, point 4.

  **The lesson for item 7 of the list above is sharper than it was written.**
  "A first deploy checked in all three engines" was on that list and was not
  done; this is exactly the failure it was there to catch, and it was found by
  a human loading the site rather than by anything in the pipeline.
- **Whether `/_pochoir/engine/*` matches as expected.** Only the trailing
  splat is documented, which is why the engine has its own directory instead of
  a `*.wasm` pattern.
- **The 404 path**, and it is the one case now wired to settle itself.
  `wrangler.jsonc` sets `not_found_handling: "404-page"` so the asset router
  answers, where `_headers` applies, rather than the no-op Worker, where
  Cloudflare documents that it does not. `make web-serve` emulates that
  locally — an unmatched path returns `dist/404.html` with the full header set
  — and every emulated 404 carries an `X-Pochoir-Emulated` header saying what
  is not being emulated. `make web-verify-live` then asserts it against the
  real origin and **fails** if the 404 is missing the security headers. So the
  first deploy answers the question without anyone remembering to ask.
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
