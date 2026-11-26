# ADR-0009: Deploy plumbing, and what attestation actually covers

- **Status:** Proposed
- **Date:** 2026-11-26
- **Deciders:** the deploy-plumbing session, under Samuel Bagattin's standing
  rule that he is asked only for product direction, spending, legal and
  irreversible calls. Decision 5 is a correction to an accepted-in-spirit
  design and is flagged for his review rather than taken unilaterally.
- **Supersedes:** nothing. It implements
  [ADR-0004](0004-build-reproducibility-and-deployed-bundle-attestation.md)
  Decisions 4 and 5 for the half of the build that exists, and **corrects
  ADR-0004 Decision 2 on one point of fact** — see Decision 5.

## Context

Nine pull requests in, `web/` had no `make` target at all. The root `Makefile`
was not the web session's to edit, so a broken web build passed CI: `make
check` ran five Rust steps and nothing that knew the site existed. At the same
time ADR-0004 had specified, in careful detail, a chain of hashes that nothing
emitted and nothing checked.

This ADR records the plumbing that closes both, and one measurement that
contradicts the design it was implementing.

Nothing here deploys. There is no Cloudflare account, no domain and no
credential; `docs/DEPLOY.md` is the runbook and lists what is missing.

## Decision 1 — `make` learns about `web/`, and the build is not done until it is attested

`web-install`, `web-build`, `web-check`, `web-serve`, `web-reproduce`,
`web-engine`, `web-deploy-check`, `web-deploy`, `web-verify-live`, `web-clean`.
`check` gains `web-check`, so CI covers the site without a second workflow and
without build logic leaving the Makefile.

The load-bearing choice is that **`web-build` is `pnpm build` *plus*
`scripts/web-attest.py`, and the bare `pnpm build` output is not deployable.**
`web/dist` straight out of Astro has no `integrity` attributes, no
`modulepreload` links and no manifest; with the `_headers` file this ADR ships
it is a blank page, not a degraded one. Making attestation a separate optional
step would mean the failure mode is "someone forgot", and the symptom would be
a white screen in production. Making it part of the build means the only way to
get a `dist` is to get an attested one.

`--frozen-lockfile` is the default everywhere, laptop included, with `FROZEN=`
to relax it deliberately. Node is pinned in `web/.node-version` and pnpm in
`package.json`'s `packageManager`, each in exactly one place, the way
`rust-toolchain.toml` owns the Rust version.

## Decision 2 — wrangler is fetched on demand, not depended upon

`pnpm dlx wrangler@4.168.0`, invoked only by the deploy targets, with
`WRANGLER_SEND_METRICS=false`.

**Measured:** wrangler 4.168.0 installs **80 packages and 268 MB**, against the
**289 packages** the entire web application's `devDependencies` currently
resolve to. Adding it to `package.json` would grow every `make check`, every CI
run and every contributor's first clone by that much, to serve a step that runs
when someone deploys. ADR-0001's instinct applies directly: a dependency we can
avoid holding, we do not hold.

The version is pinned in the `Makefile`, which is where a reviewer looking at
the deploy will read it. `WRANGLER_SEND_METRICS=false` because wrangler's
telemetry is on by default and posts to Cloudflare; a project whose claim is
that nothing leaves the device should not have its own build phoning home.

## Decision 3 — the headers live in `web/public/_headers`, comments at column 0

`public/` is copied verbatim by Astro, so the file that ships is the file in
the repository, with no copy step to forget.

Two constraints on how it is written, both of them about not trusting what we
cannot test:

- **Every comment sits at column 0 and never inside a rule body.** Cloudflare
  documents `#` comments for this format and says nothing about an indented
  one. A comment mis-parsed as a header name would break the CSP silently,
  which is the single worst outcome available here.
- **Every narrower rule detaches before it sets.** `! Cache-Control` then
  `Cache-Control:`. Cloudflare stacks all matching rules and comma-joins
  duplicates, so `/_pochoir/*` setting a second `Cache-Control` on top of
  `/*`'s would produce a joined value rather than an override.

The engine gets its own directory, `/_pochoir/engine/`, purely so that
`Content-Type: application/wasm` can be set with a trailing splat — the only
pattern form Cloudflare documents. A `*.wasm` pattern would be a guess.

`scripts/web-serve.py` reads that same file and applies it, reproducing the
stacking and the comma-joining on purpose. `astro preview` sends none of these
headers, so without it the strictest part of the site is first observable on a
deploy, which is the wrong order to find out.

## Decision 4 — the manifest, and one hex string that stands for the build

`scripts/web-attest.py` writes `dist/manifest.json`: every file with its
SHA-384 and its `integrity` form, the git commit and whether the tree was
clean, the hashes of `pnpm-lock.yaml` and `Cargo.lock`, and the build
environment.

The part worth stating is the split. `files` and `files_digest` derive from the
built bytes **and nothing else**; the `build` section — node's version, pnpm's,
rustc's — is informational and deliberately excluded from the digest. That is
what lets `make web-reproduce` compare two builds by comparing one hex string,
instead of diffing a document that can never match across two machines.

**Measured:** two builds of this tree, from two different absolute paths, with
`node_modules` reinstalled from the lock file in the second, produce the
identical `files_digest`, and `dist/` contains **zero** occurrences of the
builder's home directory. The web half of ADR-0004 Decision 1's property holds
today, without a container and without a new tool.

## Decision 5 — SRI on the entry script covers 28% of the main thread's JavaScript, and ADR-0004 Decision 2 does not say so

This is the finding this ADR exists for.

ADR-0004 Decision 2 decides to "put SRI `integrity` attributes on every
`<script>` and `<link>` the site serves", and to "additionally send
`Integrity-Policy: blocked-destinations=(script)`". Both sentences are correct
about the mechanisms and both are incomplete about this build, because Vite
code-splits: the page loads one 20 235-byte entry module which statically
imports a 50 313-byte chunk. **An ES module import carries no integrity
metadata and there is no attribute that can give it any.**

Measured, by appending one line to the imported chunk and reloading:

| Configuration | Tampered chunk | Page |
| --- | --- | --- |
| `integrity` on the entry `<script>` only | **ran** | mounted normally, no error reported |
| `Integrity-Policy: blocked-destinations=(script)` added | blocked | **blank** — the import is refused outright |
| `<link rel="modulepreload" integrity>` added for the chunk | blocked | does not mount, error in the console |
| `modulepreload` + `Integrity-Policy`, untampered | — | mounts and renders normally |

So ADR-0004 Decision 2 as written would have shipped a scheme attesting
20 235 of the 70 548 bytes the main thread executes — **28.7%** — and 0 of the
4 909 bytes the Web Worker executes, while ADR-0004's own recommended hardening
header would have taken the site down.

**Decision: `web-attest.py` walks the module graph of every script the HTML
references and emits a `<link rel="modulepreload" href=… integrity=…>` for each
chunk, and `_headers` ships `Integrity-Policy: blocked-destinations=(script)`
on the strength of that.** `--verify` then asserts the result is complete: every
reference carries a hash matching its file, every imported module has a
preload, and any JavaScript in the build that no mechanism covers is a build
failure unless it is one of two named exceptions. A chunk the import scanner
fails to see therefore fails CI instead of blanking production.

**This contradicts an accepted document and should be folded back into ADR-0004
by its owner rather than left in a second file.** It is recorded here, and not
patched into ADR-0004, because that ADR was not this session's to edit.

### The two things still unattested, named rather than glossed

- **`_pochoir/worker-*.js`, 4 909 bytes.** `new Worker(url)` has no integrity
  option, and the request's destination is `worker`, not `script`, so
  `Integrity-Policy` does not reach it either — measured: the worker loads
  normally with that header in force. This is the file the engine will run
  inside. Only the manifest and an independent rebuild can detect a swap there.
  No mechanism in any shipping browser closes this; it is a limit, not an
  oversight.
- **`_pochoir/client.*.js`, 1 011 bytes.** Astro's island bootstrap. Emitted,
  referenced by nothing, never loaded, because the island is hand-mounted. Dead
  weight rather than a hole.

## The engine path, measured against the real module rather than a stand-in

`crates/pochoir-wasm` landed while this work was in flight, so the engine half
stopped being hypothetical. `make wasm` produces **374 877 bytes**;
`make web-engine ENGINE=…` places it at
`_pochoir/engine/pochoir_wasm.94dfabd65e856e9b.wasm` and writes its SHA-384
into `web/src/engine/wasm-build.ts`.

Driven from the page's own context, against the full production header set
served by `make web-serve`:

- `fetch(url, { integrity: 'sha384-Mon3…' })` → **200**, with
  `Content-Type: application/wasm`.
- `WebAssembly.instantiateStreaming` → **instantiated**, exporting
  `memory`, `pochoir_alloc`, `pochoir_free`, `pochoir_render_pdf`,
  `pochoir_render_preview` and the five `pochoir_result_*` accessors.
  So `script-src 'self' 'wasm-unsafe-eval'` with no `'unsafe-eval'` is
  sufficient for this module — ADR-0004 Decision 5's requirement, confirmed on
  the real artefact rather than argued from the specification.
- The same fetch with a wrong hash → **`TypeError: Failed to fetch`**,
  "SRI's integrity checks failed", the resource blocked before a byte reached
  script. ADR-0004 Decision 2's fail-closed property, demonstrated.

`web/src/engine/wasm-build.ts` is committed in its *no engine* state and
regenerated by `make web-engine`. Committing it with a real hash would commit a
hash for an artefact that is not in the repository, which is precisely the
stale-hash failure Decision 4 item 4 exists to prevent.

One loose end for whoever flips `select.ts`: `abi.ts`'s own `WASM_URL` is
`/pochoir_core.wasm` — unhashed, and naming a crate that no longer exists.

## Decision 6 — an assets-only deployment is not Worker-free

Found by reading `make web-deploy-check`'s output rather than the
documentation: `wrangler deploy` for an assets-only Worker generates and
uploads `no-op-worker.js`, whose `fetch()` returns `new Response("Not found",
{ status: 404, headers: { "Content-Type": "text/html" }})`.

Cloudflare documents that `_headers` is not applied to responses generated by
Worker code. **So the 404 response on this site will carry no CSP, no
`nosniff` and no `Referrer-Policy`.** It serves a fixed eleven-byte string, so
this is a header gap rather than a risk — but ADR-0001's "no Worker code"
premise, which ADR-0004 Decision 5 restates as "with no Worker, `_headers` is
all of it", is narrower than it reads.

Recorded, not fixed. The fix is a `404.astro` plus `"not_found_handling":
"404-page"`, which keeps the response inside the asset router; that needs a
page written in French and belongs to whoever owns `web/src`. Not verified
against a live deployment: it follows from Cloudflare's documented rule and
from the artefact wrangler emits.

## Consequences

- A broken web build now fails CI. That was the hole.
- `make web-serve` makes the production policy readable on a laptop, which is
  the difference between a CSP that is believed and one that is checked.
- `make web-deploy` is one command and cannot publish an unattested bundle, but
  it cannot publish anything at all until an account exists.
- The attestation is honest about its own coverage, which is the point: a
  manifest that silently omitted 55 KB of executed JavaScript would be worse
  than no manifest, because it would be cited.
- Nothing here covers the `rustflags` field in the manifest or publishing the
  manifest as a GitHub release asset (ADR-0004 Decision 4 item 5). The wasm
  artefact's own double build is covered by `make wasm-repro`, which the
  bindings session landed separately; the deploy workflow should call it
  alongside `make web-reproduce` once the branches meet.

## Could not verify

Without a Cloudflare account, and stated as unknowns rather than assumed:
whether `_headers` comments parse as written, whether `!` detaches as
documented, whether `Integrity-Policy`, `Cross-Origin-Opener-Policy` and
`Content-Type` survive `_headers` at all, whether `/_pochoir/engine/*` matches
as expected, and the 404 behaviour of Decision 6. One `curl -I` against the
first preview settles every one of them; `docs/DEPLOY.md` carries the list.

Cross-host reproducibility of the web bundle is also unmeasured — `make
web-reproduce` compares two paths on one machine, which is the same limit
ADR-0004 records for the wasm side.

## References

- ADR-0004, Decisions 2, 4 and 5 — the specification this implements and, in
  one respect, corrects.
- Subresource Integrity: <https://www.w3.org/TR/sri-2/> — `integrity` is a
  content attribute of `link` and `script` and nothing else.
- Integrity-Policy:
  <https://w3c.github.io/webappsec-subresource-integrity/#integrity-policy-section>
- Cloudflare Workers static assets headers:
  <https://developers.cloudflare.com/workers/static-assets/headers/>
- Cloudflare Workers limits (100 rules, 2 000 characters per line):
  <https://developers.cloudflare.com/workers/platform/limits/>
