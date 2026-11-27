# ADR-0004: Build reproducibility and deployed-bundle attestation

- **Status:** Proposed
- **Date:** 2026-11-26
- **Deciders:** Samuel Bagattin
- **Supersedes:** none — this answers a requirement
  [ADR-0001](0001-stack-and-dependency-policy.md) raised and deferred

## Context

Pochoir's single claim is that the document never leaves the device. The
repository is public so that the claim can be checked. But
[ADR-0001](0001-stack-and-dependency-policy.md) already recorded the hole in
that reasoning, and it is the sharpest criticism anyone has made of this
project:

> publishing source does not, by itself, prove anything about what the user
> actually runs. **They execute whatever the CDN served them, not what is on
> GitHub.**

For a downloadable binary that gap is narrow: the user keeps the artefact and can
hash it at leisure. For a web application it is the whole problem. The bytes are
fetched fresh on every visit, from infrastructure we control, and they can be
changed silently on any deploy — or for one visitor and not the next. For a tool
whose entire proposition is that it does not exfiltrate an identity document,
**that gap is the threat model**, not a footnote to it.

ADR-0001 turned this into three requirements and left them for "a future ADR,
soon". This is that ADR. It is being written before the UI, before the wasm
binding layer and before any deployment exists, which is the cheapest possible
moment: every one of the measures below is easier to adopt as a constraint on
code that is not yet written than as a retrofit.

### What is already decided and not reopened here

- **Cloudflare Workers static assets, with no Worker code** — ADR-0001. Partly
  *because* `_headers` does not apply to Worker-generated responses, so a Worker
  would silently break our security headers.
- **A strict CSP is part of the privacy claim**, as evidence a reviewer can read
  off the response headers — ADR-0001.
- **Two wasm artefacts**, `pochoir-wasm-core` always and `pochoir-wasm-pdf`
  lazily on first PDF input — [ADR-0003](0003-pdf-input-rasterisation.md). So
  there is more than one module to attest, and one of them loads later, from
  JavaScript, on demand.
- **Single-threaded, one Web Worker, no wasm threads**, which means we need
  neither COOP nor COEP for the engine to run — ADR-0003.
- **Output is a pure function of input and parameters** —
  [ADR-0002](0002-watermark-algorithm-and-quality-modes.md). ADR-0001 notes this
  earns its keep twice: it is what makes an independent rebuild comparable at
  all.
- **Be lean with dependencies; every new one needs an ADR** — ADR-0001. Tooling
  is not exempt. Anything this ADR proposes adding is subject to it.
- **Licensing** — [ADR-0005](0005-licensing.md).

### The threat model, stated plainly

Attestation is only meaningful against named adversaries. Ours, in the order
they matter:

| # | Adversary | What they can do | Caught by the scheme below? |
| --- | --- | --- | --- |
| 1 | **Compromised build pipeline** — a GitHub Action, the runner, or the npm build tree injecting into the bundle | Add exfiltration to the shipped JS or wasm at build time | **Yes, if anyone rebuilds.** A reproducible build means an independent rebuild of the same tag produces different bytes than were deployed. This is the case the scheme is strongest against |
| 2 | **Compromised Cloudflare account or API token** | Deploy arbitrary bytes, or change `_headers`, without touching the repository | **Yes, if anyone rebuilds and compares against what is actually served.** The published hash is the tripwire; it only trips if someone looks |
| 3 | **Malicious or coerced maintainer** (including a legal order) | Ship a build that does not match the public source, or publish a hash of something other than what was deployed | **No — but it raises the cost.** They must either publish a hash that an independent rebuild contradicts, or stop publishing. Neither is quiet. See *The ceiling* |
| 4 | **Attacker between the CDN and one specific user** — enterprise TLS proxy with an installed root CA, BGP/DNS hijack with a mis-issued certificate, malicious browser extension | Serve that one user a modified bundle, leaving everyone else's intact | **No.** The victim's bytes never reach a verifier, and the same attacker can rewrite whatever page we publish the hash on. Only a copy the user holds locally helps, and only partly |
| 5 | **An honest build nobody can check** | Nothing malicious — but the served bytes cannot be shown to correspond to the source, so requirements 1–3 above are unfalsifiable | **Yes. This is the one we fix outright**, and today we fail it |
| — | **The user's own device** — malware, a malicious extension, a compromised browser | Read the document before, during or after we touch it | **No, and no scheme can.** Out of scope, and we should say so in the product rather than imply otherwise |
| — | **Cloudflare as an insider** | Same powers as #2 | Same answer as #2 |

Two things follow from reading the right-hand column. First, every "yes" is
conditional on **someone actually rebuilding**, which makes "publish the hash
somewhere a third party can find it" the load-bearing part, not the hash itself.
Second, the strongest adversary — #4, targeted substitution — is not addressed by
any amount of hashing, and we must not write copy that suggests otherwise.

### Are our builds reproducible today? Measured, not assumed

**Mostly yes, and more cheaply than expected** — but the artefact leaks the
builder's home directory, and the one case that matters most is untested.
Measured on one machine (macOS `aarch64-apple-darwin`, `rustc 1.97.0` — exactly
the `rust-toolchain.toml` pin — target `wasm32-unknown-unknown`, the repository's
release profile), against the tree at commit `42c8891`. The absolute sizes below
are therefore specific to that tree and will not match a later one; the
comparisons between them are the point, and they are what a re-measurement should
reproduce. Full method in the session note
`adr-0004-reproducibility-measurements.md`; the findings:

0. **There is no wasm artefact yet.** `make wasm` produces a **43-byte** module
   whose only export is `memory`. `pochoir-core` has no `#[no_mangle]` entry
   points and cannot have any while `lib.rs` carries `#![forbid(unsafe_code)]`,
   so the whole crate is dead-stripped out of the `cdylib`. Every measurement
   below therefore used a scratch `cdylib` outside the repository that calls
   `image_to_pdf` — **291 625 bytes** raw.
1. **`strip = true` does not remove absolute paths.** The stripped module
   contains ~20 plain-text
   `/Users/<name>/.cargo/registry/src/index.crates.io-…/png-0.18.1/src/…` paths
   and eight copies of the build directory. These are `core::panic::Location`
   strings — `file!()` in the data section, not debuginfo — so `strip` leaves
   them. The module leaks the builder's home directory before any hashing
   question arises.
2. **`trim-paths` is not available to us, but `--remap-path-scope` is.** The
   profile key `[profile.release] trim-paths` is **not stabilised in Cargo
   1.97.0** — `cargo build` refuses the manifest outright: "feature `trim-paths`
   is required […] not stabilized in this version of Cargo". It was stabilised
   for **1.101.0** (<https://github.com/rust-lang/cargo/pull/17488>, merged
   2026-10-06), with values `none | object | all`. What we *can* use today is
   `RUSTFLAGS="--remap-path-prefix=…"` together with `--remap-path-scope`, which
   is stable since 1.95.0
   (<https://github.com/rust-lang/rust/pull/147611>).
3. **The build is already path-independent.** Two byte-identical copies of the
   repository at two different absolute paths, clean `target/` each time, no
   remapping at all: **both 290 353 bytes, identical SHA-256.** Cargo hashes a
   workspace member's package id relative to the workspace root, so the checkout
   location does not reach `-Cmetadata`. The cross-host variant of this bug — the
   `host:` line of `rustc -vV` leaking into `-Cmetadata`, reported for this exact
   target in rust-lang/rust#117597 — was fixed in Cargo 1.81
   (<https://github.com/rust-lang/rust/issues/117597>,
   <https://github.com/rust-lang/cargo/pull/14107>). We are far past it.
4. **Two `RUSTFLAGS` entries close the leak and keep the identity.** With
   `--remap-path-scope=object` plus a `--remap-path-prefix` for the checkout root
   and one for `$CARGO_HOME/registry/src`, both copies produce **289 345 bytes,
   identical SHA-256, and zero host paths** (28 before, 0 after). The sysroot was
   already remapped for us by the Rust release process, to
   `/rustc/2d8144b7880597b6e6d3dfd63a9a9efae3f533d3/library/…`.
   Note that the two builds were given *different* `--remap-path-prefix` sources —
   each its own directory — and still agreed to the byte, because Cargo
   deliberately excludes that flag from the metadata hash
   (<https://github.com/rust-lang/cargo/pull/6966>, 1.37.0: "Including this flag
   in the metadata hash causes problems with reproducible builds"). Verified
   independently here: adding a no-op extra remap of a non-existent directory
   changed nothing.
5. **A first attempt at finding 3 said the opposite, and it was wrong.** It is
   recorded because the way it was wrong is a trap for whoever implements this.
   That probe was its own workspace with `pochoir-core` as a
   path dependency *outside* that workspace root; Cargo then cannot relativise the
   dependency's path, the absolute path enters `-Cmetadata`, and the crate
   disambiguator moves with the directory — `_RNvNtCs788wVNmainn_12pochoir_core…`
   against `_RNvNtCshYpo9k4QxSp_12pochoir_core…`. That renames every symbol,
   reorders the module through LLVM, and yields two artefacts of identical size
   with 11 799 bytes different. **The lesson is a constraint on the build: the
   wasm crate must be a member of the workspace that is being built.** A path
   dependency reaching outside the workspace root silently destroys
   reproducibility, and nothing in the output says so.

So: with two `RUSTFLAGS` entries we have reproducibility on one machine today,
and no leaked paths, **without a container and without a new tool**. What is left
is the discipline of finding 5, and one untested case.

**Not measurable from one machine, and decision-relevant: cross-OS and
cross-architecture reproducibility (unverified).** Whether an
`x86_64-unknown-linux-gnu` host and an `aarch64-apple-darwin` host running the
same `rustc 1.97.0` emit the same `wasm32-unknown-unknown` bytes decides whether a
hash from our Linux CI runner means anything to a verifier on a Mac. The evidence
says it should: rust-lang/rust#117597 is precisely that experiment for precisely
this target, and after the Cargo 1.81 fix the reporter confirmed consistent
output. But the Rust project publishes **no reproducibility guarantee across
hosts** — none was found — and CosmWasm, which stakes money on verifiable wasm,
states that "the native Arm version produces different wasm artifacts than the
Intel version" for its optimiser images
(<https://github.com/CosmWasm/optimizer>). That is a warning about container
images rather than about rustc, but it is the loudest one in the field. It is an
open question below and it must be answered before this ADR is accepted.

One further hazard inside our pinned toolchain, worth naming because it is
invisible: rust-lang/rust#165355 (opened 2026-11-02, fixed 2026-11-22) traced
nondeterministic release builds to LLVM's `LoopSimplify` iterating a pointer-keyed
set, observed on `rustc 1.96.0` and `1.98.1` — so **the LLVM in our 1.97.0 is
inside that window**. It was seen only with thin LTO; the reporter's `lto = "fat"`
plus `codegen-units = 1` builds held in three of three trials, which is our
configuration. Three trials on x86_64 Linux is not a guarantee, and nobody tested
wasm (<https://github.com/rust-lang/rust/issues/165355>).

## Decision 1 — Reproducibility: pin the toolchain, remap the paths, add no tooling

### Options considered

**A — Do nothing, and say so.** Keep the honest position: the source is public,
the build is not attested, there is no published hash. Free. It fails adversary
#5 outright, which makes #1 and #2 unfalsifiable, and it leaves ADR-0001's
requirement 2 unmet after that ADR promoted it to "near-term".

**B — The pinned toolchain we already have, plus two `RUSTFLAGS` entries.**
`--remap-path-scope=object` with a `--remap-path-prefix` for the checkout root
and one for `$CARGO_HOME/registry/src`, wrapped in a Makefile target, with
`--locked` so the lock file cannot drift. **Measured sufficient on this machine:**
two checkouts at different absolute paths produce byte-identical artefacts with
zero host paths embedded. No new tool, no new dependency, nothing for a verifier
to install that they do not already need to build Rust at all. It rests on the
toolchain pin being honoured, which `rust-toolchain.toml` already enforces, and
on both parties using rustup's official artefacts — a distro-built `rustc 1.97.0`
may carry a different LLVM
(<https://rustc-dev-guide.rust-lang.org/backend/updating-llvm.html>).

**C — A digest-pinned container image, built at a fixed in-container path.**
`docker run --rm -v "$PWD:/pochoir" -w /pochoir rust@sha256:<digest> make wasm`.
It pins the path, `$CARGO_HOME`, the host OS, the architecture and the C
toolchain in one move, and it is what the two production precedents for
verifiable wasm do — CosmWasm's optimiser is "a Docker build with a locked set of
dependencies to produce reproducible builds"
(<https://github.com/CosmWasm/optimizer>), and NEAR's `cargo-near` pins
`image_digest` and a fixed container build command and requires a clean git tree
(<https://docs.near.org/smart-contracts/anatomy/reproducible-builds>). Against
it: a container runtime becomes a prerequisite for verifying this project, and
the image is a third-party binary blob — the least auditable artefact in a
project that hand-rolls a JPEG encoder specifically to avoid such things. And it
does **not** pin the architecture: the same image digest run on `linux/arm64` is
a different build, which is exactly the failure CosmWasm documents — "the native
Arm version produces different wasm artifacts than the Intel version".

**D — Nix.** The strongest answer and the one the reproducible-builds world would
recommend. It loses on cost: a language and an ecosystem to learn and maintain
for a single-maintainer project, and "install Nix" is a bigger ask of a drive-by
verifier than anything else here.

### Decision

**We will make the wasm artefacts reproducible with the toolchain pin we already
have plus `--remap-path-scope=object` and two `--remap-path-prefix` entries, in a
Makefile target that CI calls and that the README documents as the verification
command. We will add no container and no new tool.**

Three constraints come with it, and they are the substance of the decision rather
than footnotes:

1. **The wasm crate must be a member of the workspace being built.** Finding 5
   above is the reason: a path dependency reaching outside the workspace root puts
   an absolute path into `-Cmetadata`, renames every symbol and destroys
   reproducibility silently. This is a real constraint on how the binding crates
   of ADR-0003 are laid out, and it should be asserted by the double build of
   Decision 4 rather than remembered.
2. **`--locked` on every release build**, so a release can never resolve a
   dependency the lock file does not name.
3. **No build step that is not measured deterministic.** `wasm-opt` and
   `wasm-bindgen` are the obvious future additions and both have history here:
   Binaryen's README states "All of the Binaryen tools are deterministic" for
   identical inputs but promises nothing across versions
   (<https://github.com/WebAssembly/binaryen>), and `wasm-bindgen` had genuine
   nondeterminism — import IDs were arena indices in binary order, which varied
   with compiler version and OS — fixed by sorting imports by name in
   rustwasm/wasm-bindgen#4190, merged 2024-10-14
   (<https://github.com/rustwasm/wasm-bindgen/pull/4190>). Either one must be
   pinned to an exact version *and* distribution, and measured, before it joins
   the build.

The reasoning, and the tests the decision had to pass:

- **It has to produce the same bytes on a machine we do not control.** Measured:
  it does, across checkout paths, on this host. The cross-host case is argued
  from rust-lang/rust#117597 and the Cargo 1.81 fix rather than measured, and is
  an open question below.
- **It has to be one command.** `make` already is the one entry point; this adds
  a target, not a procedure.
- **It must not add a dependency we would have to justify.** It adds none. That
  matters more here than it would elsewhere: ADR-0001 argues the project's
  trustworthiness from the shortness of its dependency list, and an attestation
  scheme that opened with "first install Docker" would be undercutting the
  argument it exists to support.

**Runner-up: C, the pinned container.** It lost because the measurement removed
the reason to pay for it — and because it would have been the first tool a
verifier had to trust in order to check that they need not trust us. It is not
rejected, it is *held*: it becomes the decision the moment the cross-OS
measurement below comes back negative, and it would then also need an arch pin,
not just an image digest. That is a superseding ADR, and the evidence for it is
one experiment away.

**Also noted, not chosen:** `cargo-auditable` embeds a sorted, timestamp-free
dependency list and is "specifically designed not to disrupt reproducible
builds", with wasm supported since v0.6.3
(<https://github.com/rust-secure-code/cargo-auditable>). It answers a different
question — what went in, not whether the bytes match — and its own README
declines to claim supply-chain protection. Worth revisiting if the dependency
list ever grows past the point where a reader can hold it in their head; today
that list is two crates.

## Decision 2 — Enforce the hash in the browser with `fetch` integrity, not merely publish it

A published hash that nothing checks is a document. The browser has to refuse to
run a module whose bytes are wrong, and the mechanism has to exist in Safari —
ADR-0001 already depends on Safari as the only engine that decodes HEIC for us,
so "works in Chrome" is not an answer.

### Options considered

**A — Subresource Integrity attributes.** SRI's `integrity` attribute is added
"to the list of content attributes for the link and script elements" and to those
two only; every other subresource is explicitly future work in the spec
(<https://www.w3.org/TR/sri-2/>). MDN's list is `<script>` and `<link>` with
`rel` of `stylesheet`, `preload` or `modulepreload`
(<https://developer.mozilla.org/en-US/docs/Web/Security/Subresource_Integrity>).
**So SRI does not cover a `.wasm` fetched by script at all.** Necessary for our
JS and CSS; useless on its own for the artefact that matters most. A hashing
scheme that covered the HTML and JavaScript and left the wasm unattested would be
security theatre, and this option on its own is that.

**B — `fetch(url, { integrity: "sha384-…" })`, handing the `Response` to
`WebAssembly.instantiateStreaming`.**

- `integrity` is a specified member of `RequestInit` in the WHATWG Fetch
  standard, and `Request` exposes it
  (<https://fetch.spec.whatwg.org/#requestinit>).
- Enforcement happens **before any byte reaches script**. Fetch's main-fetch
  step 23 fully reads the body, checks it against the integrity metadata, and
  only hands the response over on a match; a mismatch is a network error, and
  `fetch()` rejects a network error with a `TypeError`
  (<https://fetch.spec.whatwg.org/#main-fetch>,
  <https://fetch.spec.whatwg.org/#dom-global-fetch>). Both shipping engines
  implement it by buffering for exactly this reason — WebKit's
  `DocumentThreadableLoader` sets a delay-callbacks flag when integrity metadata
  is present, and Blink's `fetch_manager.cc` runs an integrity verifier over the
  accumulated chunks before resolving.
- `instantiateStreaming` accepts "A Response object or a promise that will
  fulfill with one", and requires the MIME type to be `application/wasm`
  (<https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/instantiateStreaming_static>).
  The integrity check is not something `instantiateStreaming` participates in; it
  has already happened inside `fetch`.
- **Browser support, which is the decisive evidence.** MDN's browser-compat data
  for `api.Request.integrity` records Chrome 46, Edge 14, Firefox 51 and
  **Safari / Safari iOS 10.1**, with no partial-implementation flag; caniuse's
  more conservative floor is Safari 11 and iOS 11.3
  (<https://caniuse.com/subresource-integrity>), and WebKit's SRI bug records the
  fetch half landing in r216553 in May 2017
  (<https://bugs.webkit.org/show_bug.cgi?id=148363>). Either way this has worked
  in all three engines for about nine years. **It is the only mechanism in this
  decision that does.**
- **The cost is the streaming overlap.** Because integrity forces the body to be
  read in full before handover, download and compilation cannot overlap;
  `instantiateStreaming` degenerates to roughly "compile a buffer". For a module
  on the order of 300 KB this is a few tens of milliseconds, and the common path
  does not load the PDF module at all (ADR-0003). We are trading a little
  start-up latency for the only enforceable guarantee available, which is the
  right trade for this product.

**C — Fetch as an `ArrayBuffer` and check it ourselves** with
`crypto.subtle.digest('SHA-384', buf)` before `WebAssembly.instantiate`.
WebCrypto is widely available, works in a Web Worker, and requires a secure
context (<https://developer.mozilla.org/en-US/docs/Web/API/Crypto/subtle>) —
which we have. It works everywhere B works and in anything older besides. The
difference is where the trust sits: the browser-enforced path **refuses to give
us the bytes**, whereas this path hands us the bytes and relies on our comparison
being correct — our encoding, our constant-time-ness, our bug. For a scheme whose
entire purpose is to be more trustworthy than our own assurances, that is the
wrong direction.

**D — The `Integrity-Policy` response header.** Real and shipping: Chrome 138,
Firefox 145 (reporting partial), Safari 26
(<https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Integrity-Policy>).
It makes integrity *mandatory* for a destination, which closes a real hole — an
attacker who can rewrite our HTML could otherwise simply delete the `integrity`
attributes.

> **Amended 2026-11-27.** "Shipping in Safari 26" was read off a compatibility
> table and taken to mean the header is usable there. It is not, for a
> code-split bundle. WebKit enforces the policy but does not credit a
> `modulepreload` link's `integrity` to the module import it covers, so every
> imported chunk is refused. Measured, not inferred — see *The ceiling*, point
> 4. A compatibility table says a feature exists; it does not say the feature
> agrees with the other engines about what satisfies it. **But its destinations are only `script` and `style`**; the spec says
"The possible values for it are \"script\" and \"style\""
(<https://w3c.github.io/webappsec-subresource-integrity/#integrity-policy-section>).
There is no `fetch` or wasm destination, so it cannot be made to cover our
modules. Complementary, not a substitute.

**E — Signature-based SRI (Ed25519).** The right shape of answer: the server
signs responses, the page names a public key as `ed25519-<base64>` in `integrity`,
and keys can be rotated. It is a **WICG Draft Community Group Report**, explicitly
"not a W3C Standard nor … on the W3C Standards Track"
(<https://wicg.github.io/signature-based-sri/>), and support is **Chrome 141
only** — not Firefox, not Safari. Unusable for us today.

### Decision

**We will load every wasm module with
`fetch(url, { integrity: "sha384-…" })` and pass the resulting `Response` to
`WebAssembly.instantiateStreaming`, and we will put SRI `integrity` attributes on
every `<script>` and `<link>` the site serves.** We will additionally send
`Integrity-Policy: blocked-destinations=(script)` so that scripts without
integrity metadata are refused rather than silently allowed.

> **Amended 2026-11-27, second sentence only.** That header now ships as
> `Integrity-Policy-Report-Only`. Sending the enforcing form made the site a
> blank page in every version of Safari, for the reason given in *The ceiling*,
> point 4. The first sentence is untouched and still holds: the `integrity`
> attributes and the `fetch` integrity on the engine are enforced by every
> engine, including Safari. What was lost is only the refusal of a script that
> arrives carrying no metadata at all. Restore the enforcing form when WebKit
> 326340 is fixed.

This is chosen because it is specified, because the check happens before our code
can touch the bytes, and above all because it works in Safari — which rules out
every other candidate that would have been stronger on paper. *(The Safari claim
holds for the `integrity` attributes and for `fetch` integrity, which were what
this comparison turned on. It does not hold for the `Integrity-Policy` header,
as the amendment above records.)* The streaming loss
is accepted deliberately and should be recorded in the first performance
measurement, not forgotten.

Two mechanical consequences to carry into implementation: the modules must be
served as `Content-Type: application/wasm` or `instantiateStreaming` rejects; and
SRI requires CORS for cross-origin requests
(<https://www.w3.org/TR/sri-2/>), which is a non-issue for us because everything
is same-origin — and `connect-src 'self'` keeps it that way.

**Runner-up: C**, the manual digest. It lost on where the trust sits, not on
compatibility — it is in fact the more portable option. It becomes the right
answer if we ever find an engine where `fetch` integrity is unenforced for
same-origin requests, and it remains a legitimate belt-and-braces addition if a
reviewer wants it.

**Revisit when:** signature-based SRI ships in two engines. A signature over the
bundle, verified against a key the page names, is strictly better than a hash the
page names, because it survives the page being regenerated. That is the single
biggest available improvement to the root-of-trust problem below, and it is
blocked only on browser support.

## Decision 3 — The root of trust, and the ceiling we cannot pass

Decision 2 has a hole in it, and naming it is the most important paragraph in
this document. If the JavaScript carries the wasm's expected hash, and the server
can swap the JavaScript, the scheme is circular: an attacker who can change the
module can change the hash that guards it. Hashing the HTML does not help,
because the HTML comes from the same place. **The root of trust cannot live
inside the thing it is rooting.**

CryptPad hit exactly this in 2018 and wrote it down honestly, which is why it is
the most useful precedent for a project our size. Its chain went HTML →
`sboot.js` → `version.txt` → signed manifest → app, with `sboot.js`'s own hash
embedded in cached HTML and a `localStorage` counter to block rollback — and the
author stated the limitation plainly: if the root HTML is compromised it can
alter or discard the whole chain of trust
(<https://blog.cryptpad.org/2018/06/15/Signing-CryptPad/>). It never left an
experimental branch.

### Options considered

**A — Published release hashes: a git tag, a GitHub release, the manifest as an
asset.** The cheap answer, and it does the one thing that matters most: it puts a
copy of the hash with a party that is not Cloudflare, so a Cloudflare compromise
cannot rewrite it and vice versa. Collusion, or a compromise of the maintainer's
GitHub account, defeats both at once. Costs nothing beyond Decision 4's manifest
step and asks a verifier to do nothing but read. Its weakness, which should be
stated rather than glossed: **a git tag is a mutable ref and a release can be
edited or deleted, so this is a statement, not an append-only log** — there is no
public witness of what the tag pointed at last week. *(That is reasoning about how
git and GitHub refs work rather than a cited claim.)*

**B — A signed manifest, with a key we hold.** Strictly more than A: it binds the
manifest to a key rather than to an account. It is also only as good as the key
custody, and for a single maintainer that means a key on a laptop or in GitHub's
secret store — the latter being the same trust boundary as the pipeline the
signature is meant to attest. It adds a key to rotate, a key to lose, and a
revocation story we would have to invent. Against adversary #3 it does nothing
at all: a coerced maintainer has the key.

**C — A transparency log.** Structurally the right answer to A's weakness: an
append-only log makes "we published a different hash last week" detectable rather
than deniable, which is what turns a hash into a commitment.

- **GitHub artifact attestations** are the cheapest real on-ramp. They bind a
  named artifact and digest to the workflow run — repository, commit SHA,
  triggering event, from the OIDC token claims — as SLSA build provenance in
  in-toto format, and for **public repositories they are free and are signed via
  the Sigstore Public Good instance, written to the public Rekor log**
  (<https://docs.github.com/en/actions/concepts/security/artifact-attestations>,
  <https://github.com/actions/attest-build-provenance>). A verifier needs GitHub
  CLI ≥ 2.49.0 and `gh attestation verify <file> -R owner/repo`.
  **The decisive caveat:** this attests *"this file came out of this workflow on
  this commit"*. It says nothing about the bytes a browser executed, which is
  the gap we are actually trying to close. It also anchors in GitHub, which for a
  GitHub-hosted project is a different party but not an independent one.
- **Sigstore/Rekor directly** can log a bare artifact hash (`hashedrekord`), but
  the durability is wrong for a claim meant to outlive a release: the stated SLO
  is 99.5% *availability*, with no durability or retention guarantee, and shards
  rotate — each gets its own `logYEAR-rev.rekor.sigstore.dev` URL, the previous
  instance is frozen shortly after, and clients must not hardcode the URL
  (<https://blog.sigstore.dev/rekor-v2-ga/>,
  <https://docs.sigstore.dev/logging/overview/>). **Do not assume a Rekor entry is
  readable in five years.**
- **Certificate Transparency as a vehicle** — Mozilla designed it
  (<https://wiki.mozilla.org/Security/Binary_Transparency>) and the implementation
  bugs are `RESOLVED/INCOMPLETE`. Nobody appears to be shipping it. *(That it was
  abandoned is an inference from the bug resolutions, not a Mozilla statement.)*

**D — Third-party mirroring and independent rebuilders.** The only option that
genuinely closes the loop, because it takes us out of the chain: someone else
rebuilds from the tag and publishes their own hash. Debian's model, via
`rebuilderd` — whose backends are Arch, Debian, Tails, Fedora and Alpine, i.e.
Linux distribution repositories, with the one public instance serving Arch
packages (<https://github.com/kpcyrd/rebuilderd>). **There is no hosted rebuilder
a web or wasm project can submit to**, and `reproducible-builds.org` has nothing
for web bundles. So in practice this option means "make rebuilding one command,
and hope somebody does it" — which is precisely what Decision 1 enables and
precisely why the verification command belongs in the README.

**E — An offline copy the user keeps.** The only option that touches adversary #4,
targeted substitution, because it stops the bytes being re-fetched from us.

**The PWA half of this is marketing and we must not sell it.** A service worker
is an update mechanism as much as a cache. The spec's Handle Fetch triggers a
soft update for every non-subresource request — which navigations are — and a
registration is stale once more than 86 400 seconds have passed since its last
update check (<https://w3c.github.io/ServiceWorker/>, §2.3 and Handle Fetch step
13.1.2.1). Most browsers **ignore caching headers when checking the worker
script**, so a long `max-age` does not pin anything, and the worker counts as
updated if it is byte-different
(<https://web.dev/articles/service-worker-lifecycle>). `skipWaiting()` plus
`clients.claim()` let a replacement take over already-open pages immediately
(<https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API/Using_Service_Workers>).
So a service worker buys **offline availability and a bounded detection window**,
not a pin, and the origin can ship replacement code silently.

**The other half is real.** A downloaded build the user hashes once and opens
from `file://`, and the native CLI we already have, are genuine pins, because
nothing is re-fetched. This is the honest answer to "what do I do if I think
someone is targeting me", and it costs us nothing we were not already building.

**F — A browser extension that checks for the user.** Two exist and both are
instructive.

- **Meta's Code Verify** publishes a manifest of "the hashes of all the
  JavaScript/CSS files in a given release" to a Cloudflare audit endpoint, and the
  extension compares what the page loaded against it
  (<https://engineering.fb.com/2022/03/10/security/code-verify/>,
  <https://blog.cloudflare.com/cloudflare-verifies-code-whatsapp-web-serves-users/>).
  It is **alive** — version 4.2.0, updated September 2026, and the Cloudflare
  endpoint is still in current source — so "it was abandoned" is not the reason to
  pass on it. The reasons are three. It covers **JavaScript and CSS, not
  WebAssembly**, which is the artefact we most need attested. Cloudflare rebuilds
  nothing, so it proves *"everyone got the bytes Meta published a hash for"*, not
  that those bytes came from any published source — it is consistency, not
  source-to-binary verification. And the adoption number is the one that settles
  it: **734 daily users on Firefox** (Mozilla's own add-on API) and roughly 30 000
  on Chrome, for a system protecting WhatsApp, Messenger and Instagram on the web.
- **WEBCAT**, from the Freedom of the Press Foundation, is closer to what we would
  want — a Firefox extension that blocks code before execution unless it matches a
  signed, publicly logged manifest
  (<https://freedom.press/tech/news/introducing-webcat-web-based-code-assurance-and-transparency/>).
  It is explicitly alpha, flagged experimental on its own listing, and has **32
  daily users**. Enrollment also appears to run through a permissioned consensus
  chain operated by a limited set of organisations, so it is probably not
  something we could join unilaterally *(unverified — the site-operator
  documentation could not be read)*.

Writing and maintaining an extension is far beyond this project's means in any
case, and the user numbers say that even the best-resourced attempt reaches
nobody.

### Decision

**We will publish the manifest as a GitHub release asset on a release tag, and
also at a stable path on the site; we will put the one-command verification
procedure in the README; and we will not hold a signing key or run a transparency
log at this stage.** Option A, with D as the thing the README exists to enable
and E's downloadable build as the honest answer for anyone who thinks they are a
target. (Whether the tag itself is GPG-signed is an open question below, not
something this decision assumes: a signed tag needs a key, and declining to hold
one means the tag is authenticated by the GitHub account and nothing else.)

The reasoning is a judgement about where the marginal honesty lies. Going from
nothing to A is a large step: it creates a record held by a party that is not our
CDN, which is what makes adversaries #1 and #2 detectable at all. Going from A to
B or C is a much smaller step *for this project*, because every stronger root
still terminates in the maintainer's GitHub account — and it buys that increment
at the price of a key to manage or a log whose operator makes no durability
promise. ADR-0001's rule applies to tooling as much as to crates, and the honest
reading of it here is that ceremony we would not maintain is worse than a simpler
scheme we would.

**Runner-up: C**, GitHub artifact attestations. It lost on being nearly free
rather than actually free — it asks a verifier to install and trust the GitHub
CLI, and it anchors where we already anchor. It wins the moment either of two
things is true: the project acquires users who would plausibly run
`gh attestation verify`, or we want to make a claim about *who built* a release
rather than about *what is in* it. Both are plausible within a year, and adopting
it later costs one workflow step, so nothing is lost by waiting.

**Revisit when WAICT ships.** Web Application Integrity, Consistency and
Transparency is Mozilla's Firefox Security Team with Cloudflare, the Freedom of
the Press Foundation and Meta: sites bind their client-side code to an integrity
manifest and commit that manifest to a publicly auditable, witness-signed log,
and a site that opts into enforcement gets **the browser to reject code that has
not been publicly logged**. There is a prototype behind a pref in Firefox
Nightly, and the drafts are public
(<https://hacks.mozilla.org/2026/05/trustworthy-javascript-for-the-open-web/>,
<https://blog.cloudflare.com/improving-the-trustworthiness-of-javascript-on-the-web/>,
<https://github.com/waict-wg>). Two things make it the one to watch rather than
one more proposal: its stated next steps include **extending SRI to WebAssembly**,
which is the exact hole Decision 2 works around; and it is being designed with the
CDN we already use. If it ships, we adopt rather than build. Mozilla's own framing
is the right one to borrow — it makes previously hidden attacks observable and
attributable. That is detection, not prevention, and nobody involved pretends
otherwise.

### The ceiling

**For a web application served from infrastructure we control, there is no
mechanism that fully closes the gap between published source and executed bytes,
and this ADR does not claim one.** The limit is structural, not a gap in our
scheme, and it has four parts. The first three are about trust; the fourth is
about what browsers will actually enforce, and it was added after the first
deploy measured it.

1. **Every root of trust we can offer is served by us or signed by a key we
   hold.** The browser has no independent notion of "the Pochoir that the
   repository describes". It knows only what the origin sent it, and the origin is
   us. A hash in the JavaScript, a manifest at a well-known path, a signature —
   all of them arrive through the channel they are meant to police. Everything in
   the field that genuinely escapes this moves the anchor out of the page
   entirely: an extension, or the browser itself. Both are out of our reach, and
   the extension route reaches, empirically, a few hundred people.
2. **Verification is retrospective and voluntary.** The scheme catches *silent,
   persistent* substitution — a bad build that stays bad long enough for someone
   to rebuild and compare. It does not catch a substitution aimed at one user, or
   one that lasts an hour, because the victim's bytes never reach a verifier. For
   adversary #4 the published hash is worth nothing. It is also worth recording
   the general form of this: reproducible builds on their own bring no benefit to
   end users, because the benefit depends on verifiers who actually check
   (<https://arxiv.org/html/2510.02251v1>) *(read via summary, not in full)*.
3. **A maintainer who is compelled, or compromised, holds every key.** A
   published hash does not stop them. What it changes is what they can do
   *quietly*: to ship a bundle that does not match the public source they must
   either publish a hash an independent rebuild would contradict, or stop
   publishing hashes. Both are visible to anyone paying attention. The threat is
   not hypothetical — a court order plus a technical assistance order and a gag
   can compel an ordinary-looking update from the developer themselves, and the
   ACLU's own list of defences against it is code signing, binary transparency,
   reproducible builds and mirrorable distribution
   (<https://www.aclu.org/how-malicious-software-updates-endanger-everyone>); the
   UK's s.253 technical capability notices are the same shape with statutory
   secrecy
   (<https://privacyinternational.org/long-read/5547/about-our-challenge-against-uks-secret-powers-qa-tcns-and-apple>).
   **But we should not overclaim here either: no source we found argues that
   published reproducible-build hashes legally constrain such an order.** The
   nearest is Rebecca Wexler's "disclosure by design" — the argument that
   compelling someone to keep publishing a now-false assurance is compelled
   speech and should draw strict scrutiny — and she labels the legal consequences
   speculative and notes no court has decided it
   (<https://www.yalelawjournal.org/forum/warrant-canaries-and-disclosure-by-design>).
   The defensible claim is the technical one: a public record converts a silent
   targeted swap into a detectable and attributable one, which raises the cost and
   the political risk of the order. Anything stronger is speculation.

4. **Two of the files we ship cannot be covered by any integrity mechanism in
   a shipping browser, and a third is covered in only one engine.** This is the
   most concrete part of the ceiling and the easiest to overstate, so it is
   given as bytes. The main thread executes 70 548 bytes of JavaScript across
   two files and the Web Worker a further 4 909.

   | File | Bytes | Chromium | WebKit / Safari | Gecko / Firefox |
   | --- | --- | --- | --- | --- |
   | `index.astro_…js`, the entry | 20 235 | enforced, `integrity` attribute | enforced, `integrity` attribute | enforced, `integrity` attribute |
   | `index-client.*.js`, imported by it | 50 313 | enforced, via `modulepreload` | **not checked** | **not checked** |
   | `worker-*.js` | 4 909 | **not checked** | **not checked** | **not checked** |

   The worker is a limit of the platform and always was: `new Worker(url)` has
   no integrity option, and the request's destination is `worker`, not
   `script`, so `Integrity-Policy` never reaches it either. Nothing in any
   shipping engine closes it.

   The imported chunk is the one we thought we had closed and had not, outside
   Chromium. **A static ES `import` carries no integrity metadata and there is
   no attribute that can give it any** (ADR-0009 Decision 5). The workaround is
   a `<link rel="modulepreload" href=… integrity=…>` for each chunk, which
   Chromium credits to the import. **Measured 2026-11-27 in Mobile Safari on
   iOS 26.5: WebKit does not. A chunk whose bytes were altered so that they no
   longer match the hash on its `modulepreload` link was fetched and executed,
   and the page mounted normally.** **Firefox 160.0.1 runs the tampered chunk
   too.** So outside Chromium the enforced share of the main thread is 20 235
   of 70 548 bytes — 28.7% — which is exactly the figure ADR-0009 Decision 5
   believed it had fixed, and it was fixed in one engine of three.

   Worse, the same gap made the enforcing form of `Integrity-Policy`
   unshippable — in the one engine that enforces it at all. Firefox does not:
   the enforcing header neither blocks the chunk nor breaks the page there,
   which matches MDN's "reporting partial" for Gecko. WebKit *does* enforce
   `blocked-destinations=(script)`, and it
   *does* honour an `integrity` attribute on a `<script>` the HTML names — a
   build with that attribute deleted is refused there. What it will not do is
   accept a `modulepreload` link as the metadata for the import it covers, so
   under the enforcing header every code-split chunk is a script with no
   metadata and is refused outright. The live site was a blank page in Safari
   from the first deploy until this was measured. WebKit bug 326340. The header
   now ships as `Integrity-Policy-Report-Only`; `web/public/_headers` carries
   the measurement and the condition for restoring it.

   The honest statement to a reader is therefore much narrower than Decision
   2's: *in Chromium the browser refuses 93.5% of the JavaScript we ship if its
   bytes do not match; in Safari and Firefox, 26.8%; the Web Worker is covered
   in none of the three, and for all of it the manifest and an independent
   rebuild are the only check.* One engine of three does what Decision 2
   assumed all of them did. Both engines still enforce `fetch` integrity on the wasm engine
   itself, which is the largest single artefact and the one Decision 2 was
   mainly about.

The only two things that genuinely close the loop need someone other than us: a
third party who rebuilds and publishes independently, and a user who keeps a copy
they verified once and runs it without re-fetching. That is why the one-command
rebuild and the CLI matter more than any mechanism we could add to the website.

So the honest version of the claim is: *the code is public, the build is
reproducible, the hash of what we deployed is published where we do not control
the only copy, and the browser refuses to run a module whose bytes do not match.
If you want to check, here is the one command. If someone can compel us, or can
attack you specifically, none of this saves you — and for that, use the CLI
offline.* That is less than a guarantee and more than anyone else in this space
offers. The rest of our documents are written this way for a reason: the
overstated version would be the first thing a hostile reader disproved.

## Decision 4 — What CI has to do (described here, implemented elsewhere)

This ADR deliberately writes no workflow. What it fixes is the contract the
workflow must satisfy, so that the implementation is a transcription rather than
a design exercise. `make` stays the only entry point: every step below is a
Makefile target that CI calls, per the project's standing rule.

1. **Build twice and compare.** The release job builds the wasm artefacts with
   the flags of Decision 1, moves the output aside, builds again from clean, and
   compares byte-for-byte. Better still, it builds the second time from a
   *different checkout directory*, since that is the variable Decision 1's
   finding 5 shows can silently reintroduce path-dependence. A mismatch is a hard
   failure. This is what stops us
   from publishing a hash for a build that is not in fact reproducible — the
   failure mode where we claim a property we have quietly lost. It roughly
   doubles the release build time, which for a project this size is tens of
   seconds.
2. **Refuse to deploy on a mismatch.** The comparison gates the deploy step, not
   merely a report. A release that cannot reproduce itself does not ship.
3. **Emit a manifest, and treat it as the deliverable.** A single file listing
   every served path with its SHA-384, plus the git commit, the exact `rustc`
   version string, the `RUSTFLAGS` used and the `Cargo.lock` hash — everything a
   verifier needs to reconstruct the build, and nothing they would have to take
   on trust. SHA-384 rather than
   SHA-256 because that is what the `integrity` attribute will carry anyway, so
   one digest serves both purposes.
4. **Generate the `integrity` values from that manifest, never by hand.** The
   HTML's SRI attributes and the wasm loader's `integrity` strings are build
   outputs. A hand-maintained hash is a hash that will be stale on the first
   hurried release, and a stale hash fails closed — the app simply stops working,
   which is the one failure mode a user will certainly notice and we will
   certainly be blamed for.
5. **Publish the manifest where we do not control the only copy.** As a GitHub
   release asset on the tag, and at a stable path on the site itself. The second
   is convenience; only the first is evidence, and even then only because GitHub
   is a different party from Cloudflare. Decision 3 is about how far that gets us.
6. **Verify what is actually served, after deploying.** Fetch each asset from the
   production origin and compare against the manifest. This is the only step that
   can catch adversary #2 — a deploy that did not come from CI — and it is the
   step most likely to be skipped because it is the only one that touches the
   network after a green build. It should also run on a schedule, not only at
   release: a bundle swapped a week after deploy is exactly the attack that a
   release-time check cannot see.

   **Amended 2026-11-27, after the check failed a deploy that had succeeded.**
   Run 45985959359 uploaded a correct bundle; the check then reported two
   content-addressed chunks 404 and `index.html` mismatched, and failed the
   job. The origin was correct a minute later. It had raced Cloudflare's
   deployment propagation — documented behaviour, not a platform fault: a
   version owns an asset manifest, "deployment changes take up to a few
   seconds to reach every location", and until they have, "locations can use
   different deployment configurations", so a request landing on the previous
   version asks for a path that version never had
   (<https://developers.cloudflare.com/workers/versions-and-deployments/version-skew/>).

   This matters out of proportion to the inconvenience. A gate that cries wolf
   is a gate people switch off, and this is the only step in the scheme that
   sees what a visitor receives. A flaky version of it is worse than none,
   because it trains everyone to read a red verify as noise. So three things
   are fixed here rather than left to the implementation:

   - **The unit of comparison is the whole manifest, and a retry restarts it.**
     A partial match is not success. The property worth asserting is that every
     path matched *at the same moment*; patching up individual paths across
     passes would assert something weaker while reading as though it asserted
     this.
   - **"Not there yet" and "wrong" must be reported differently, and only one
     of them may wait.** A non-200 on a manifest path, or a stale body at a
     path whose name carries no content hash, is consistent with propagation
     and is retried until a deadline. A mismatch at a **content-addressed**
     path is not, at any point: content-hashed names exist so that different
     contents have different paths, so skew makes such a path absent, never
     different. Different bytes there are bytes we never built, and the check
     fails on the first pass. Past the deadline, a still-stale body is a
     failure (the deploy did not take) while a still-absent path is
     *inconclusive* — the check neither confirmed the bundle nor caught
     anything.
   - **The deadline is a stated number with a stated provenance, not a tuned
     one.** 120 seconds: twenty to forty times Cloudflare's documented "a few
     seconds", above the whole interval the one observed race leaves open
     (1.7s to about a minute), and short enough that a broken deploy is red
     within two minutes of the upload. Deliberately not sized for the reported
     cases of a deploy serving a stale asset manifest for far longer; in that
     state red is the correct answer.

   **And the claim a green run licenses is narrower than "the deploy is
   verified".** It is: *every path in the manifest was served, with matching
   bytes, to this client, from whichever Cloudflare location it reached, within
   N seconds of the upload.* The same Cloudflare page is explicit about the
   limit, and it is a limit on any check of this shape rather than on ours:
   "Waiting a fixed time or testing one location does not prove that every
   location has the change." Retrying buys freedom from a race; it buys no
   knowledge of the other three hundred cities. The script says so in its own
   output, because a verification step whose summary overstates itself is how
   an honest scheme acquires a dishonest reputation.

   The step keeps failing the job. The false red was fixed at its cause, not by
   downgrading the alarm: a warning-only live check is a log line, and item 6
   exists precisely because this step is the one most likely to be ignored.
7. **Read the platform's own deployment record while doing so.** Cloudflare's
   versions API returns, per version, `metadata.created_on`, `author_email` and a
   `metadata.source` whose enum distinguishes `wrangler`, `dash`, `api` and
   `terraform` from `workersci`
   (<https://developers.cloudflare.com/api/resources/workers/subresources/scripts/subresources/versions/methods/list/>).
   A deploy from a laptop therefore leaves no trace in the repository but a clear
   one in the platform, and the scheduled check can assert that the live version
   came from CI. Note the retention: **30 days, except the 100 most recent
   versions**
   (<https://developers.cloudflare.com/workers/configuration/versions-and-deployments/>),
   so this record is a tripwire, not an archive — which is one more reason the
   manifest has to be published somewhere we do not control.

Two constraints on step 1 that are easy to lose. It must build the **workspace**
containing the wasm crate, because Decision 1's finding 5 makes the crate's
position in the workspace load-bearing. And it must run with `--locked`.

**What this costs.** Every release gains a second build, a manifest to publish, a
post-deploy check, and a scheduled job. None of it is clever; all of it is
upkeep, and it is upkeep on the part of the project least likely to be exercised
day to day, which means it will rot silently unless step 1 fails loudly when it
does.

## Decision 5 — The CSP, as evidence rather than as a control

ADR-0001 already decided that a strict CSP is part of how the privacy claim is
made checkable: "a `connect-src 'self'` that forbids outbound requests is
evidence a reviewer can read off the response headers". This decision records
what it has to contain, what it cannot do, and the two things about Cloudflare
that will bite.

### `'wasm-unsafe-eval'` is mandatory, and we will not pair it with `'unsafe-eval'`

Any WebAssembly path needs `'wasm-unsafe-eval'` in `script-src`. It is defined in
CSP Level 3 — §2.3.1's source-list grammar and §4.5 "Integration with
WebAssembly", whose `EnsureCSPDoesNotBlockWasmByteCompilation` throws a
`WebAssembly.CompileError` in enforce mode
(<https://w3c.github.io/webappsec-csp/>). A `script-src 'self'` with no wasm
keyword **blocks `WebAssembly.instantiateStreaming`** regardless of where the
bytes came from; Firefox's implementation bug lists that exact sink
(<https://bugzilla.mozilla.org/show_bug.cgi?id=1740263>), and a real-world
report of it biting is
<https://bugzilla.mozilla.org/show_bug.cgi?id=1770468>. Support: Chrome 97, Edge
97, Firefox 102, **Safari 16 and Safari iOS 16**
(<https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/script-src>).

`'unsafe-eval'` also permits wasm compilation and, per MDN, "overrides any
occurrence of `'wasm-unsafe-eval'`", so listing both is the standard
older-browser fallback. **We will not do it.** It would re-enable `eval()` for
every visitor in order to serve browsers older than Safari 16, and `eval()` in a
page that handles identity documents is precisely the thing the CSP is supposed
to be evidence against. Users on engines older than that get a clear message, not
a weakened policy. This is a capability regression we are choosing, in the same
spirit as ADR-0001's HEIC decision.

This is easy to forget until the module fails to instantiate, and it fails at
instantiation rather than at fetch — which is why the first deploy must be
checked in all three engines rather than assumed.

### The shape of the policy

`default-src 'none'` and then allow only what is needed, with `connect-src
'self'` as the line a reviewer is meant to read, `object-src 'none'`,
`base-uri 'none'`, `form-action 'none'` and `frame-ancestors 'none'`.

**And we should be honest about what it is not.** CSP has no shipped directive
that restricts top-level navigation, so a script that wanted to exfiltrate could
navigate rather than fetch. `connect-src 'self'` is therefore strong evidence
about the code's *intent* and a real barrier to the commonest exfiltration
shapes; it is not a seal on the origin. The product copy should claim the former.

### Cloudflare mechanics, with the two traps

- **`_headers` is the only mechanism we have, and that is by design.** Workers
  static assets supports a `_headers` file
  (<https://developers.cloudflare.com/workers/static-assets/headers/>), and
  ADR-0001's premise is confirmed verbatim by Cloudflare: "Custom headers defined
  in the `_headers` file are not applied to responses generated by your Worker
  code." With no Worker, `_headers` is all of it — which is the trade ADR-0001
  made deliberately.
- **Limits:** 100 header rules per file, and **2 000 characters per line**
  including the header name — which is the practical ceiling on a single CSP
  value, reached long before the 128 KB total-response-header cap
  (<https://developers.cloudflare.com/workers/platform/limits/>). A policy
  carrying per-script hashes could hit it; ours should not, because SRI carries
  the hashes instead.
- **Trap 1: matching rules stack and duplicate headers are comma-joined.** Two
  rules that both set `Content-Security-Policy` produce one comma-joined value,
  which the browser treats as two policies and intersects. Use `!` to detach
  before re-setting in a narrower rule. Cloudflare documents the same hazard for
  `Cache-Control`.
- **Trap 2, and it is the serious one: during a gradual rollout, "HTML and its
  assets may come from different versions"**
  (<https://developers.cloudflare.com/workers/static-assets/>). With Decision 2
  in place that is not a cosmetic problem — HTML from version N+1 carrying
  `integrity` for assets that resolve to version N's bytes **fails closed**, and
  the user sees a broken app. The fix is content-addressed asset filenames so
  that each version's assets live at distinct paths and an old HTML keeps
  resolving to the bytes it names. Cloudflare does not do this for us: its asset
  manifest is keyed by request path and filename hashing is the build's job. So
  **fingerprinted asset filenames are a requirement of this ADR, not a
  performance nicety.**
- **Defaults worth knowing:** Cloudflare attaches `Cache-Control: public,
  max-age=0, must-revalidate` and an `ETag` that "is a hash of the static asset
  file". Both are overridable from `_headers`, and fingerprinted assets should
  carry `immutable`. The ETag is not an attestation — the algorithm is not
  documented and the value comes from Cloudflare, not from us — but it is worth
  not confusing the two when reading response headers.

### COOP and COEP

ADR-0003 removed the need for both: a single-threaded wasm rasteriser in one Web
Worker needs neither header, and `COEP: require-corp` would make every
cross-origin subresource opt in via CORP. We will set `COOP: same-origin` as
ordinary isolation hygiene and **will not set COEP**.

The question ADR-0001 left open — whether Cloudflare permits COEP in `_headers` —
**is still open, and this ADR does not close it.** Neither the Workers
static-assets headers page nor the Pages equivalent mentions COEP or COOP at all;
there is no Cloudflare statement permitting or forbidding either, and no
confirmed report of COEP being stripped. Third-party documentation asserts the
`_headers` recipe works, and the failures that turn up in Cloudflare's community
forum are attributable to caching or to pattern matching rather than to stripping
— but that is hearsay, not evidence. Settling it needs one `curl -I` against a
deployed preview, which costs nothing the day there is a deployment. It is
recorded as unverified rather than resolved because **it does not block us**: we
have decided not to need COEP.

## Consequences

### What this gets us

Adversary #5 is closed: it becomes possible, for the first time, for someone
other than us to check whether the bytes the CDN serves correspond to the public
source. Adversaries #1 and #2 acquire a tripwire that trips the moment anybody
looks, and the browser — not our documentation — refuses to run a wasm module
whose bytes are wrong. The artefact stops naming the maintainer's home directory.
And it costs no new dependency, which matters more here than it would elsewhere:
ADR-0001 argues the project's trustworthiness from the shortness of its
dependency list, and an attestation scheme opening with "first install Docker"
would undercut the argument it exists to support.

It also gives the CLI a second reason to exist. Decision 3 concludes that the
only genuine pin is a copy the user keeps, which means the native binary stops
being a convenience for developers and becomes the answer we give to anyone who
believes they are being targeted specifically. That is worth knowing before the
CLI's scope is decided.

### What this costs us

Every release gains a second build, a manifest to publish, a post-deploy check
and a scheduled job — upkeep on the part of the project exercised least often,
which means it will rot quietly unless the double build fails loudly. The wasm
start-up loses the download/compile overlap, because `fetch` integrity forces the
body to be buffered before handover. Asset filenames must be content-addressed,
not for performance but because Cloudflare may serve HTML and assets from
different versions mid-rollout and SRI fails closed when they disagree. We take
on a standing obligation to keep published hashes correct, and a stale
`integrity` string does not degrade gracefully — it breaks the app for everyone.
Users on engines older than Safari 16 lose the tool entirely, because we decline
to pair `'wasm-unsafe-eval'` with `'unsafe-eval'`. And the build acquires a
constraint nothing in its output will remind us of: the wasm crate must be a
workspace member.

There is also a cost that is not technical. Publishing a hash invites the
question "and did you check it?", to which the honest answer will usually be "no,
nobody did". We should not present the scheme as load-bearing while it is latent.

### What this forecloses

Deploying by hand from a laptop, as a normal practice: the manifest and the
post-deploy check mean little if a laptop can deploy, so the Cloudflare token
stops being something we hold locally. Shipping any build step that is not
measured deterministic — which is a live constraint on `wasm-opt` and
`wasm-bindgen`, and on the PDF renderer of ADR-0003, whose reproducibility
nobody has checked. A path dependency reaching outside the workspace. And
minified-but-unmapped output, which ADR-0001 had already ruled out.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| Cross-OS wasm output turns out not to match, so a verifier on a different host gets a mismatch and concludes we are lying | **Medium** | **High** | Measure before accepting this ADR. If it fails, Decision 1's runner-up — a digest-pinned container, plus an architecture pin — becomes the decision, in a superseding ADR |
| A stale or mismatched `integrity` string ships and the app fails closed for everyone | **Medium** | **High** | Hashes generated from the manifest, never by hand (Decision 4 step 4); content-addressed filenames so a mid-rollout version skew cannot cause it; post-deploy check |
| The LLVM `LoopSimplify` nondeterminism in 1.97.0's LLVM bites under fat LTO after all | Low | **High** | The double build catches it on the release that hits it, and a toolchain bump past the fix is the remedy. It was observed only with thin LTO |
| Someone adds a wasm crate as a path dependency outside the workspace and reproducibility silently dies | **Medium** | Medium | Asserted by the double build, documented in the build target, and written down in Decision 1 |
| `wasm-opt` or `wasm-bindgen` joins the build without being measured, and breaks the double build | Medium | Medium | Pin exact version *and* distribution; measure first. Neither is in the build today |
| Nobody ever verifies, so the scheme is decorative | **High** | Low | Accepted, and stated rather than hidden. The hash still constrains what can be done quietly — see *The ceiling* |
| We publish a hash and a reader takes it as a guarantee against a targeted attack | Medium | **High** | The product copy must state the ceiling in a user's words, not only in this ADR |
| Cloudflare's 30-day / 100-version retention means a past deploy cannot be audited | Medium | Low | The manifest is published on GitHub too, which is the copy that is meant to outlive the platform's record |
| The reproducibility work displaces the watermark, which is the actual product | Medium | Medium | This is a decision, not a sprint. Implementation order: bindings and watermark first, attestation at the first real deploy |

## Open questions

- [ ] **Needs measurement before acceptance — does a Linux x86_64 host produce
      the same `wasm32-unknown-unknown` bytes as this macOS aarch64 host, with the
      same `rustc 1.97.0` and the flags of Decision 1?** Everything measured here
      is one machine. The evidence says it should hold, but the Rust project
      publishes no guarantee. *This is one build and one `shasum`, and it decides
      whether Decision 1 stands or its runner-up takes over — so it should happen
      before this ADR is accepted, not after.*
- [ ] **Needs decision from Samuel — where is the manifest published, and is a
      transparency log worth it?** Recommendation in Decision 3. It is here
      because the stronger options add a key to manage or a service to depend on,
      and that is a judgement about how much ceremony this project should carry.
- [x] **No signing key. Decided by Samuel Bagattin on 2026-11-26.**

      Hashes are published to a GitHub release, anchoring trust where this
      project already anchors it, and nothing is signed.

      Two reasons, and the second is the one that settles it. **A key we hold
      is a key that can be compelled or stolen** — for a single maintainer on a
      laptop, custody is the whole of the security, and there is no custody
      story here better than "it is on the laptop". And **a signature would
      imply a guarantee this ADR says plainly we cannot give**: it would read,
      to anybody who did not read this document, as an attestation that a
      trusted party vouches for the deployed bundle at a point in time, which
      is exactly the claim ADR-0002 Decision 7 already refuses to make about
      the watermark for the same reason. Adding ceremony that overstates what
      we know is worse than publishing a hash that states it exactly.

      This closes one question and not the others: the manifest's location, the
      deploy token, the wording of the ceiling, the Linux measurement and the
      COEP question are all still open, and **this ADR stays `Proposed`**.

      *As raised:* do we hold a signing key at all? A signed manifest is only as
      good as the key custody, and for a single maintainer on a laptop that is a
      real question rather than a formality. Declining to hold one is a
      defensible answer, and it is the recommendation unless the answer to the
      previous question is "yes, sign it".
- [ ] **Needs decision from Samuel — does the deploy token move to CI only?**
      Decision 4's post-deploy check is close to meaningless if a laptop can
      deploy. This forecloses a convenience a solo maintainer will miss, so it is
      his call and not ours.
- [ ] **Unresolved — how is the ceiling stated to users?** This ADR states it for
      us. The product has to say it in a sentence a non-specialist reads
      correctly, without overclaiming and without burying it. That is a wording
      decision and a product decision.
- [ ] **Should the manifest record which paths are content-addressed?** Item 6's
      fast-fail rule turns on that distinction, and `verify-live.py` currently
      *infers* it from the filename — a hash-shaped token on an extension we
      emit hashed. The inference is checked against the real manifest and gets
      all eleven paths right today, and it fails safe (a path it does not
      recognise is treated as stable, so it is retried rather than failed), but
      it is a heuristic sitting under a security check, which is the wrong place
      for one. The build knows the answer for certain. Moving it into
      `manifest.json` is a `manifest_version` bump and a change to
      `web-attest.py`, which is why it was not done in the same change as the
      retry loop.
- [ ] **Still open from ADR-0001, and not closed here — does Cloudflare honour
      `Cross-Origin-Embedder-Policy` in `_headers`?** No Cloudflare documentation
      says either way. It does not block us, because ADR-0003 removed the need for
      COEP, so this stays open deliberately. One `curl -I` against the first
      deployed preview settles it.

## Could not verify

- **Cross-OS and cross-architecture reproducibility of
  `wasm32-unknown-unknown` output.** Not testable on one machine; Docker was
  installed but its daemon was not running. The first open question above.
- **That `rustc 1.97.0` from rustup embeds the same LLVM commit on every host.**
  Inferred from Rust's pinned-submodule model
  (<https://rustc-dev-guide.rust-lang.org/backend/updating-llvm.html>); no
  document states it. `rustc -vV` on both machines settles it, and a distro-built
  rustc may differ.
- **Whether `trim-paths` (Cargo 1.101) would change any of this.** Not stabilised
  on our toolchain, so it could not be tried. Measure the day it ships; it would
  replace the `RUSTFLAGS` pair with a profile key, which is tidier but not
  different in effect as far as we can tell.
- **Whether `--remap-path-scope` is excluded from Cargo's metadata hash** the way
  `--remap-path-prefix` documentedly is. Both our builds passed the same value,
  so it did not matter here; it would matter if two verifiers passed different
  scopes.
- **Whether `wasm-opt` and `wasm-bindgen` are deterministic** for fixed versions,
  and whether `wasm-opt` is deterministic across host OS and architecture.
  Binaryen's README claims determinism only for identical inputs; `wasm-bindgen`
  has no equivalent statement at all. Neither is in the build today.
- **Any empirical cross-browser test** of `fetch(url, {integrity})` feeding
  `WebAssembly.instantiateStreaming`. The claim that the check applies, and that
  the streaming overlap is lost, rests on Fetch's main-fetch step 23 plus Blink
  and WebKit source — not on a test run or a vendor statement, and MDN's
  `instantiateStreaming` page does not mention integrity at all. Confirm by hand
  in all three engines before relying on it.
- **Whether Cloudflare honours COEP in `_headers`**, and whether there is any
  authoritative list of headers Cloudflare silently drops. Neither is documented.
- **WEBCAT's enrollment model.** Decision 3 states that enrollment runs through a
  permissioned consensus chain operated by a limited set of organisations, which
  is why we treat it as unavailable to us. The site-operator documentation could
  not be read; the claim rests on a repository mirror and an archived copy. If
  enrollment turns out to be open, WEBCAT deserves a second look.
- **That Mozilla abandoned binary transparency via certificate transparency.**
  Inferred from implementation bugs resolved `INCOMPLETE`, not from any Mozilla
  statement.
- **The claim that reproducible builds alone bring no benefit to end users**, used
  in the ceiling's point 2, comes from a 2025 preprint read via summary rather
  than in full (<https://arxiv.org/html/2510.02251v1>). It is cited as support
  for a point this ADR argues independently, not as its basis.
- **Whether a git tag's mutability matters in practice** — Decision 3 option A
  calls the tag a statement rather than a log. That is reasoning about how git and
  GitHub refs work, not a sourced claim.
- **Whether any Cloudflare API can return the asset *contents* of a past
  version**, which is what a third-party "what was served on date X" audit would
  need. Not documented as possible.
- **Exact release dates for Safari 26 and Firefox 145**, cited for
  `Integrity-Policy` support. Version numbers come from MDN's compat data; the
  dates were not confirmed against a primary release page. *(2026-11-27: the
  Safari entry turned out to be the more important problem. The compat table
  was right that the header exists there and wrong, by omission, about whether
  it is usable — see the ceiling, point 4. Firefox is still unmeasured by us.)*
- ~~**Firefox.**~~ Measured on 2026-11-27, Firefox 160.0.1: it does not credit
  a `modulepreload` link's `integrity` to the import, and does not enforce
  `Integrity-Policy` at all. Folded into the ceiling's point 4. What remains
  untested is every engine older than the three measured, and any mobile
  browser that is not WebKit.

## References

- ADR-0001, *Stack and dependency policy* — [0001-stack-and-dependency-policy.md](0001-stack-and-dependency-policy.md)
- ADR-0002, *Watermark algorithm and output quality modes* — [0002-watermark-algorithm-and-quality-modes.md](0002-watermark-algorithm-and-quality-modes.md)
- ADR-0003, *PDF input rasterisation* — [0003-pdf-input-rasterisation.md](0003-pdf-input-rasterisation.md)

### Reproducibility

- Cargo book, profiles (`strip`) — <https://doc.rust-lang.org/cargo/reference/profiles.html#strip>
- rustc book, codegen options (`-C strip`) — <https://doc.rust-lang.org/rustc/codegen-options/index.html#strip>
- rustc book, remapping source paths — <https://doc.rust-lang.org/rustc/remap-source-paths.html>
- rustc book, command-line arguments (`--remap-path-prefix`) — <https://doc.rust-lang.org/rustc/command-line-arguments.html>
- rust-lang/rust#75799, registry paths in release binaries — <https://github.com/rust-lang/rust/issues/75799>
- rust-lang/rust#147611, stabilise `-Zremap-path-scope` (1.95.0) — <https://github.com/rust-lang/rust/pull/147611>
- rust-lang/cargo#17488, stabilise `profile.trim-paths` (1.101.0) — <https://github.com/rust-lang/cargo/pull/17488>
- rust-lang/cargo#6966, exclude `--remap-path-prefix` from the metadata hash (1.37.0) — <https://github.com/rust-lang/cargo/pull/6966>
- rust-lang/rust#117597, wasm32 output differs between macOS and Ubuntu — <https://github.com/rust-lang/rust/issues/117597>
- rust-lang/cargo#14107, make `-Cmetadata` host-independent (1.81.0) — <https://github.com/rust-lang/cargo/pull/14107>
- rust-lang/rust#165355, LLVM `LoopSimplify` nondeterminism — <https://github.com/rust-lang/rust/issues/165355>
- rustc dev guide, updating LLVM — <https://rustc-dev-guide.rust-lang.org/backend/updating-llvm.html>
- reproducible-builds.org, Rust — <https://reproducible-builds.org/docs/rust/>
- rust-secure-code, supply-chain security: build — <https://rust-secure-code.github.io/rust-supply-chain-security/build.html>
- WebAssembly/binaryen (determinism claim in the README) — <https://github.com/WebAssembly/binaryen>
- rustwasm/wasm-bindgen#4190, make output deterministic — <https://github.com/rustwasm/wasm-bindgen/pull/4190>
- CosmWasm/optimizer (Docker-based reproducible wasm; the ARM caveat) — <https://github.com/CosmWasm/optimizer>
- NEAR, reproducible builds (`image_digest`, NEP-330) — <https://docs.near.org/smart-contracts/anatomy/reproducible-builds>
- rust-secure-code/cargo-auditable — <https://github.com/rust-secure-code/cargo-auditable>

### Integrity in the browser

- Subresource Integrity (W3C Working Draft, 20 March 2026) — <https://www.w3.org/TR/sri-2/>
- Integrity-Policy, SRI Editor's Draft §3.8 — <https://w3c.github.io/webappsec-subresource-integrity/#integrity-policy-section>
- WHATWG Fetch, `RequestInit` — <https://fetch.spec.whatwg.org/#requestinit>
- WHATWG Fetch, main fetch (step 23, the integrity check) — <https://fetch.spec.whatwg.org/#main-fetch>
- MDN, Subresource Integrity — <https://developer.mozilla.org/en-US/docs/Web/Security/Subresource_Integrity>
- MDN, `RequestInit` — <https://developer.mozilla.org/en-US/docs/Web/API/RequestInit>
- MDN, `WebAssembly.instantiateStreaming()` — <https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/instantiateStreaming_static>
- MDN, `Integrity-Policy` — <https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Integrity-Policy>
- MDN, `Crypto.subtle` — <https://developer.mozilla.org/en-US/docs/Web/API/Crypto/subtle>
- MDN, CSP `script-src` — <https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/script-src>
- CSP Level 3, §4.5 Integration with WebAssembly — <https://w3c.github.io/webappsec-csp/>
- caniuse, Subresource Integrity — <https://caniuse.com/subresource-integrity>
- WebKit bug 148363, Implement Subresource Integrity — <https://bugs.webkit.org/show_bug.cgi?id=148363>
- Bugzilla 1740263, Firefox `'wasm-unsafe-eval'` — <https://bugzilla.mozilla.org/show_bug.cgi?id=1740263>
- Bugzilla 1770468, `instantiateStreaming` blocked by CSP in the wild — <https://bugzilla.mozilla.org/show_bug.cgi?id=1770468>
- Signature-based Integrity (WICG draft) — <https://wicg.github.io/signature-based-sri/>

### Root of trust and prior art

- CryptPad, *Signing CryptPad* (2018) — <https://blog.cryptpad.org/2018/06/15/Signing-CryptPad/>
- Meta Engineering, *Code Verify* — <https://engineering.fb.com/2022/03/10/security/code-verify/>
- Cloudflare, verifying the code WhatsApp Web serves — <https://blog.cloudflare.com/cloudflare-verifies-code-whatsapp-web-serves-users/>
- `facebookincubator/meta-code-verify` — <https://github.com/facebookincubator/meta-code-verify>
- Freedom of the Press Foundation, WEBCAT — <https://freedom.press/tech/news/introducing-webcat-web-based-code-assurance-and-transparency/>
- Mozilla Hacks, *Trustworthy JavaScript for the open web* (WAICT) — <https://hacks.mozilla.org/2026/05/trustworthy-javascript-for-the-open-web/>
- Cloudflare, improving the trustworthiness of JavaScript on the web (WAICT) — <https://blog.cloudflare.com/improving-the-trustworthiness-of-javascript-on-the-web/>
- WAICT working group drafts — <https://github.com/waict-wg>
- GitHub, artifact attestations — <https://docs.github.com/en/actions/concepts/security/artifact-attestations>
- `actions/attest-build-provenance` — <https://github.com/actions/attest-build-provenance>
- Sigstore, Rekor v2 GA — <https://blog.sigstore.dev/rekor-v2-ga/>
- Sigstore, logging overview (the 99.5% availability SLO) — <https://docs.sigstore.dev/logging/overview/>
- Mozilla wiki, binary transparency — <https://wiki.mozilla.org/Security/Binary_Transparency>
- `kpcyrd/rebuilderd` — <https://github.com/kpcyrd/rebuilderd>
- Service Worker specification (soft update, staleness) — <https://w3c.github.io/ServiceWorker/>
- web.dev, the service worker lifecycle — <https://web.dev/articles/service-worker-lifecycle>
- MDN, using service workers — <https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API/Using_Service_Workers>
- ACLU, how malicious software updates endanger everyone — <https://www.aclu.org/how-malicious-software-updates-endanger-everyone>
- Privacy International, on UK technical capability notices — <https://privacyinternational.org/long-read/5547/about-our-challenge-against-uks-secret-powers-qa-tcns-and-apple>
- Rebecca Wexler, *Warrant Canaries and Disclosure by Design*, 124 Yale L.J. F. 158 — <https://www.yalelawjournal.org/forum/warrant-canaries-and-disclosure-by-design>

### Hosting

- Cloudflare, Workers static assets — <https://developers.cloudflare.com/workers/static-assets/>
- Cloudflare, Workers static assets: headers — <https://developers.cloudflare.com/workers/static-assets/headers/>
- Cloudflare, Workers platform limits — <https://developers.cloudflare.com/workers/platform/limits/>
- Cloudflare, versions and deployments (retention, rollback) — <https://developers.cloudflare.com/workers/configuration/versions-and-deployments/>
- Cloudflare, list script versions API (`metadata.source`) — <https://developers.cloudflare.com/api/resources/workers/subresources/scripts/subresources/versions/methods/list/>
