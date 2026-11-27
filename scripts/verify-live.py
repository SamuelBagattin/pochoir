#!/usr/bin/env python3
"""Fetch what an origin is actually serving and compare it with the manifest.

ADR-0004 Decision 4 item 6. This is the only check in the whole scheme that can
catch adversary #2 — a deploy that did not come from CI — because every other
step reasons about bytes we produced rather than bytes a visitor receives. It
is also the step most likely to be skipped, being the only one that touches the
network after a green build, which is why it is a Makefile target and a
workflow step rather than a note in a runbook.

It should run on a schedule as well as at release. A bundle swapped a week
after deploy is exactly the attack a release-time check cannot see. On a
schedule, pass `--deadline 0`: there is no deploy in flight, so there is
nothing to wait for and a single pass is the honest check.

What it checks, per file in the manifest:
  * the response status;
  * the SHA-384 of the body against the manifest;
  * for the engine module, that Content-Type is exactly `application/wasm`,
    because `WebAssembly.instantiateStreaming` rejects anything else.

And twice more, on paths rather than files:

  * the site root, for the security headers the deploy is supposed to be
    carrying — `_headers` is uploaded as part of the bundle, and a silently
    dropped header is indistinguishable from a working deploy until someone
    reads a response;
  * **an unmatched path**, which is the one check that settles a question
    nothing local can. `wrangler.jsonc` sets `not_found_handling: "404-page"`
    so that a 404 is answered by the asset router, where `_headers` applies,
    rather than by the no-op Worker wrangler uploads, where Cloudflare
    documents that it does not. That reasoning is sound and unobserved. If the
    Worker answers first, this request comes back with none of the headers
    below, and that is a real finding rather than a cosmetic one — it would
    mean a page on our origin served without a Content-Security-Policy.


Deployment propagation, and why this check retries
--------------------------------------------------

Run 45985959359 uploaded a correct bundle and then failed this check: two
content-addressed paths came back 404, `index.html` came back with the
*previous* deploy's bytes, and a hand check a minute later found the origin
entirely correct. Nothing was wrong with the deploy. The check had raced
Cloudflare's propagation and cried wolf — and a gate that cries wolf is a gate
people switch off, which for the only mechanism that would catch a bundle
swapped outside CI is worse than having no gate at all.

Cloudflare documents the mechanism, and it accounts for every symptom of that
run (<https://developers.cloudflare.com/workers/versions-and-deployments/version-skew/>):

  * "Deployment changes take up to a few seconds to reach every location on
    Cloudflare's network. … Until the change reaches every location, locations
    can use different deployment configurations."
  * A version's **asset manifest** maps paths to files, and "uploading a file
    does not add it to other versions' manifests". So a request that lands on
    a location still serving the old version asks for a path that version has
    never heard of, and "a missing file can return a 404 response" — our two
    404s, on exactly the two newest content-addressed chunks.
  * The same skew in the other direction is our `index.html` mismatch: a
    location still on the old version serving the old HTML.
  * It also explains the oddest line in that run, which was easy to miss: the
    engine `.wasm` was fetched twice, 1.6 seconds apart, and came back 200 and
    correct the first time and 404 the second. Two requests, two locations, two
    deployment configurations. (That duplicate fetch is gone: the engine block
    below now reuses the observation from the file pass, so a single pass makes
    a single request per path and has a chance of being coherent.)

So the check retries, and the unit of retry is **the whole manifest**. A
partial match is not success: the claim worth making is that every path matched
*at the same moment*, so a retry restarts the comparison rather than patching up
the paths that failed last time.

Three severities, because "not there yet" and "wrong" deserve different words:

  PENDING   a non-200 on a manifest path, a missing header, an odd 404 probe.
            Consistent with propagation. Retried; if the deadline passes with
            only these, the check is INCONCLUSIVE (exit 3) — it did not prove
            the deploy good and it did not catch anything bad.
  STALE     a 200 whose bytes hash differently at a path whose name does not
            carry a content hash (`index.html`, `404.html`). Consistent with a
            location still serving the previous version. Retried; if the
            deadline passes it is a FAILURE (exit 1), because "still serving
            the old bundle two minutes later" is not propagation, it is a
            deploy that did not take.
  WRONG     not explainable by propagation at any point, so it fails on the
            first pass without waiting:
              - a 200 whose bytes hash differently at a *content-addressed*
                path. Cloudflare's own advice is content-hashed filenames
                precisely so that "different file contents then have different
                paths"; version skew makes such a path absent, never different.
                Different bytes at that path are bytes we never built.
              - a platform control file (`_headers`) readable at its own path.

Whether a path is content-addressed is inferred from its name (see
`is_content_addressed`), because the manifest does not record it. That is the
one soft spot in the scheme and the fail-safe direction is deliberate: a path
we fail to recognise as content-addressed is treated as stable, i.e. retried,
which risks waiting out the deadline but never fails a good deploy. Moving the
fact into `manifest.json` at build time, where it is known rather than guessed,
is the right long-term answer and is recorded as an open question in ADR-0004.


What a green run does and does not prove
----------------------------------------

The honest claim is narrow, and narrower than "the deploy is verified":

    every path in the manifest was served, with matching bytes, to this
    client, from whichever Cloudflare location this client reached, within
    N seconds of the upload.

Not "the deploy is verified", and not "every location serves this bundle".
Cloudflare's own page is explicit about the second, in a sentence worth quoting
in full because it bounds what any check of this shape can do: "Waiting a fixed
time or testing one location does not prove that every location has the
change." A deadline buys us freedom from a race we were losing. It buys no
knowledge about the other 300-odd cities, and this script does not claim any.

Standard library only — urllib, not a client we would have to justify.

Exit codes:
  0  every path matched in a single coherent pass
  1  the origin is serving bytes we did not build (WRONG now, or STALE at the
     deadline)
  2  usage
  3  inconclusive: the deadline passed with only propagation-shaped findings
Note that `make` collapses all of these to its own exit 2; the codes are for a
direct caller, and the printed classification is the signal a human reads.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

# Files the platform consumes rather than serves; asking for them should 404
# and a 200 would mean the control file is publicly readable at its own path.
NOT_SERVED = {"_headers", "_redirects", "_routes.json"}

# Headers whose absence from a production response means the deploy is not
# carrying the policy this project's claims rest on.
REQUIRED_HEADERS = (
    "content-security-policy",
    "x-content-type-options",
    "referrer-policy",
    "cross-origin-opener-policy",
)

USER_AGENT = "pochoir-verify-live"

# Severities. See the module docstring.
WRONG = "WRONG"
STALE = "STALE"
PENDING = "PENDING"

EXIT_OK = 0
EXIT_WRONG = 1
EXIT_USAGE = 2
EXIT_INCONCLUSIVE = 3

# How long to tolerate propagation, in seconds, and the gaps between passes.
#
# 120 seconds. Where it comes from, since a number chosen to make one run pass
# would be worthless:
#
#   * Cloudflare documents deployment propagation as taking "up to a few
#     seconds to reach every location". 120s is some twenty to forty times
#     that, so a deploy that has not converged by then has not hit the
#     documented behaviour — it has hit something else, and something else is
#     what this check exists to notice.
#   * It covers the whole interval the one observed race leaves open, with
#     margin. In run 45985959359 the check failed 1.7s after its step started
#     and a hand check "about a minute later" was clean, so that day's true
#     convergence time lies somewhere in (1.7s, ~60s]. 120s is the smallest
#     round number above the top of that interval. This is one observation and
#     is stated as one; it is not a distribution.
#   * There are credible reports of Workers deploys serving a stale asset
#     manifest for far longer than a few seconds (cloudflare/workers-sdk
#     #16658 and #16447). Those are unverified user reports, not platform
#     behaviour, and deliberately NOT what the deadline is sized for: if we
#     are in that state, red is the correct outcome and waiting ten minutes to
#     say so would only hide it.
#   * It is short enough that a genuinely broken deploy is red within two
#     minutes of the upload. A gate nobody waits for is a gate nobody runs.
#
# The observed race it has to cover was ~1.7s wide: the failing check ran
# 1.7s after the step started and a hand check about a minute later was clean.
DEFAULT_DEADLINE = 120.0
BACKOFF = (2.0, 4.0, 8.0, 15.0)

# A filename token that is a content hash: at least eight characters of the
# base64url-ish alphabet Vite and our own engine generator use, carrying either
# a digit or both cases, which is what distinguishes `DfUqYJwv` from a word
# like `document`. Only on extensions we actually emit content-addressed; a
# `.html` is never treated as content-addressed, which is the whole point of
# the distinction. A new hashed asset type must be added here, and until it is,
# it is treated as stable — slower to fail, never wrong to pass.
_HASH_TOKEN = re.compile(r"(?:^|[./-])([A-Za-z0-9_-]{8,32})\.[A-Za-z0-9]+$")
_HASHED_EXTENSIONS = {
    "js", "mjs", "cjs", "css", "wasm",
    "woff", "woff2", "ttf", "otf",
    "png", "jpg", "jpeg", "webp", "avif", "gif", "ico", "map",
}


def is_content_addressed(rel: str) -> bool:
    """True when this path's name carries a content hash.

    Such a path cannot legitimately serve bytes other than the ones whose hash
    named it, so a mismatch there is never propagation. See the docstring.
    """
    extension = rel.rsplit(".", 1)[-1].lower() if "." in rel else ""
    if extension not in _HASHED_EXTENSIONS:
        return False
    match = _HASH_TOKEN.search(rel)
    if not match:
        return False
    token = match.group(1)
    has_digit = any(c.isdigit() for c in token)
    mixed_case = any(c.islower() for c in token) and any(c.isupper() for c in token)
    return has_digit or mixed_case


class Finding:
    """One problem with one observation, and how propagation explains it."""

    __slots__ = ("severity", "text")

    def __init__(self, severity: str, text: str) -> None:
        self.severity = severity
        self.text = text


def fetch(url: str, timeout: float) -> tuple[int, bytes, dict[str, str]]:
    """Status, body and headers. Status 0 means the request did not complete.

    A connection reset or a timeout used to raise out of this function and take
    the whole run with it, which is one more way to fail a deploy that is fine.
    It is now an observation like any other, and a retryable one.
    """
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            headers = {k.lower(): v for k, v in response.headers.items()}
            return response.status, response.read(), headers
    except urllib.error.HTTPError as exc:
        headers = {k.lower(): v for k, v in exc.headers.items()} if exc.headers else {}
        return exc.code, exc.read(), headers
    except (urllib.error.URLError, TimeoutError, OSError):
        return 0, b"", {}


def run_pass(
    origin: str, manifest: dict, timeout: float
) -> tuple[list[Finding], list[str]]:
    """One complete comparison of the origin against the manifest.

    Returns the findings and the lines to print. Nothing is printed here: the
    caller decides which passes are worth showing in full, because ten passes
    of a thirteen-path table is noise that hides the one pass that matters.
    """
    findings: list[Finding] = []
    lines: list[str] = []
    checked = 0

    # Every observation this pass made, so that nothing is fetched twice. Two
    # requests for one path can land on two locations with two deployment
    # configurations, which is how the old script managed to report the engine
    # as both present and absent in the same run.
    seen: dict[str, tuple[int, bytes, dict[str, str]]] = {}

    def observe(rel: str) -> tuple[int, bytes, dict[str, str]]:
        if rel not in seen:
            seen[rel] = fetch(f"{origin}/{rel}", timeout)
        return seen[rel]

    for rel, meta in sorted(manifest.get("files", {}).items()):
        status, body, headers = observe(rel)

        if rel in NOT_SERVED:
            if status == 200:
                findings.append(
                    Finding(WRONG, f"{rel}: served with 200; it should not be reachable")
                )
                lines.append(f"  200  {rel}  SERVED — it must not be")
            else:
                lines.append(f"  {status:>3}  {rel}  (not served, as expected)")
            continue

        checked += 1
        if status != 200:
            shown = "err" if status == 0 else status
            findings.append(
                Finding(
                    PENDING,
                    f"{rel}: HTTP {shown}"
                    + (
                        " — absent from the version this request reached"
                        if is_content_addressed(rel)
                        else ""
                    ),
                )
            )
            lines.append(f"  {shown:>3}  {rel}  PENDING")
            continue

        got = hashlib.sha384(body).hexdigest()
        if got != meta["sha384"]:
            addressed = is_content_addressed(rel)
            findings.append(
                Finding(
                    WRONG if addressed else STALE,
                    f"{rel}: body does not match the manifest\n"
                    f"      manifest {meta['sha384']}\n"
                    f"      served   {got}\n"
                    + (
                        "      this path carries a content hash, so no version "
                        "of this site ever served\n"
                        "      these bytes at it. This is not propagation."
                        if addressed
                        else "      this path has no content hash, so a previous "
                        "deploy could legitimately\n"
                        "      be serving it during propagation."
                    ),
                )
            )
            lines.append(f"  200  {rel}  {'MISMATCH' if addressed else 'STALE'}")
        else:
            lines.append(f"  200  {rel}  ok ({len(body)} bytes)")

        if rel.endswith(".wasm"):
            ctype = headers.get("content-type", "")
            if ctype.split(";")[0].strip() != "application/wasm":
                findings.append(
                    Finding(
                        PENDING,
                        f"{rel}: Content-Type is {ctype!r}; "
                        f"WebAssembly.instantiateStreaming requires application/wasm",
                    )
                )

    # The engine. Its hash is asserted against the dedicated `engine` block
    # rather than the `files` entry, because that block is what the loader's
    # `integrity` string is generated from — but the bytes are the ones the
    # pass already fetched, not a second request.
    engine = manifest.get("engine")
    if engine and "path" in engine:
        rel = engine["path"]
        fresh = rel not in seen
        status, body, headers = observe(rel)
        if fresh:
            checked += 1
        if status != 200:
            shown = "err" if status == 0 else status
            findings.append(Finding(PENDING, f"{rel}: HTTP {shown} (engine)"))
        elif hashlib.sha384(body).hexdigest() != engine["sha384"]:
            findings.append(
                Finding(
                    WRONG,
                    f"{rel}: engine bytes do not match the manifest's engine block",
                )
            )
        elif headers.get("content-type", "").split(";")[0].strip() != "application/wasm":
            findings.append(
                Finding(
                    PENDING,
                    f"{rel}: Content-Type is "
                    f"{headers.get('content-type')!r}, not application/wasm",
                )
            )

    # The security headers, read off the root. All PENDING: `_headers` ships
    # inside the bundle and propagates with it, so a location still on the old
    # version can answer with the old header set.
    status, _body, headers = fetch(f"{origin}/", timeout)
    lines.append("  response headers on /:")
    for name in REQUIRED_HEADERS:
        value = headers.get(name)
        if value is None:
            findings.append(Finding(PENDING, f"/: {name} is absent from the response"))
            lines.append(f"    {name}: ABSENT")
        else:
            lines.append(f"    {name}: {value[:110]}{'…' if len(value) > 110 else ''}")
    csp = headers.get("content-security-policy", "")
    if "'unsafe-inline'" in csp or "'unsafe-eval'" in csp:
        findings.append(
            Finding(PENDING, "/: the served CSP contains 'unsafe-inline' or 'unsafe-eval'")
        )

    # The unmatched path. A random segment, so it cannot collide with a real
    # asset or with anything a future router learns to answer.
    probe = f"{origin}/pochoir-verify-live-{hashlib.sha256(origin.encode()).hexdigest()[:12]}"
    status, body, headers = fetch(probe, timeout)
    lines.append(f"  unmatched path -> HTTP {'err' if status == 0 else status}")
    if status != 404:
        findings.append(
            Finding(
                PENDING,
                f"an unmatched path returned HTTP {'err' if status == 0 else status}, "
                f"not 404. With not_found_handling it should be 404; a 200 would "
                f"mean every typo renders as the application",
            )
        )
    missing_on_404 = [name for name in REQUIRED_HEADERS if name not in headers]
    if missing_on_404:
        findings.append(
            Finding(
                PENDING,
                "the 404 response is missing "
                + ", ".join(missing_on_404)
                + ". This is the case wrangler.jsonc records as unverified: the "
                "uploaded no-op Worker answered instead of the asset router, and "
                "_headers does not reach Worker-generated responses",
            )
        )
    else:
        lines.append(
            "    carries the security headers: the asset router answered, "
            "not the no-op Worker"
        )
    ctype_404 = headers.get("content-type", "").split(";")[0].strip()
    if status == 404 and ctype_404 != "text/html":
        findings.append(
            Finding(
                PENDING,
                f"the 404 response has Content-Type {ctype_404!r}; the built "
                f"404.html should be served as text/html",
            )
        )

    lines.append(f"  {checked} file(s) compared")
    return findings, lines


def report(findings: list[Finding], stream) -> None:
    for severity in (WRONG, STALE, PENDING):
        group = [f for f in findings if f.severity == severity]
        if not group:
            continue
        print(f"  {severity}:", file=stream)
        for finding in group:
            print(f"    {finding.text}", file=stream)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="verify-live.py")
    parser.add_argument("origin", help="https://pochoir.app, or a preview URL")
    parser.add_argument("--manifest", type=Path, default=Path("web/dist/manifest.json"))
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument(
        "--deadline",
        type=float,
        default=DEFAULT_DEADLINE,
        help=(
            "seconds to tolerate Cloudflare deployment propagation before "
            f"giving up (default {DEFAULT_DEADLINE:.0f}). Use 0 for a single "
            "pass, which is what a scheduled run wants: nothing is propagating, "
            "so there is nothing to wait for."
        ),
    )
    args = parser.parse_args(argv[1:])

    origin = args.origin.rstrip("/")
    if not origin:
        print("verify-live: no origin given", file=sys.stderr)
        return EXIT_USAGE
    if args.deadline < 0:
        print("verify-live: --deadline cannot be negative", file=sys.stderr)
        return EXIT_USAGE
    if not args.manifest.is_file():
        print(f"verify-live: {args.manifest} is missing; build first", file=sys.stderr)
        return EXIT_USAGE

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))

    print(f"verify-live: {origin}")
    print(f"  manifest files_digest {manifest['files_digest']}")
    if args.deadline > 0:
        print(
            f"  tolerating up to {args.deadline:.0f}s of deployment propagation; "
            f"each pass compares the whole manifest from scratch"
        )
    else:
        print("  single pass, no propagation tolerance (--deadline 0)")

    started = time.monotonic()
    attempt = 0
    findings: list[Finding] = []
    lines: list[str] = []

    while True:
        attempt += 1
        findings, lines = run_pass(origin, manifest, args.timeout)
        elapsed = time.monotonic() - started

        if attempt == 1:
            for line in lines:
                print(line)

        if any(f.severity == WRONG for f in findings):
            # Not explainable by propagation. Waiting cannot make it better and
            # a slow alarm is a worse alarm.
            if attempt > 1:
                for line in lines:
                    print(line)
            print(
                f"\nverify-live: FAILED on pass {attempt} after {elapsed:.1f}s — "
                f"the origin is serving bytes we did not build.",
                file=sys.stderr,
            )
            print(
                "  This is not propagation, so the check did not wait for the "
                "deadline.",
                file=sys.stderr,
            )
            report(findings, sys.stderr)
            return EXIT_WRONG

        if not findings:
            if attempt > 1:
                for line in lines:
                    print(line)
            print(
                f"\nverify-live: every path in the manifest matched in one pass, "
                f"{elapsed:.1f}s after this check started"
                + (f" (pass {attempt})" if attempt > 1 else "")
                + "."
            )
            print(
                "  What that proves: this client, reaching one Cloudflare "
                "location, was served the\n"
                "  bundle we built. It does not prove every location serves it — "
                "Cloudflare's own\n"
                "  docs note that testing one location does not establish that."
            )
            return EXIT_OK

        remaining = args.deadline - (time.monotonic() - started)
        gap = BACKOFF[min(attempt - 1, len(BACKOFF) - 1)]
        if remaining <= 0.5:
            break
        gap = min(gap, remaining)
        pending = sum(1 for f in findings if f.severity == PENDING)
        stale = sum(1 for f in findings if f.severity == STALE)
        print(
            f"  pass {attempt} at {elapsed:.1f}s: {pending} pending, {stale} stale; "
            f"retrying the whole manifest in {gap:.1f}s "
            f"({remaining:.0f}s of deadline left)"
        )
        time.sleep(gap)

    # The deadline passed. Show the last pass in full, then say which kind of
    # failure this was: the origin is wrong, or we could not tell.
    elapsed = time.monotonic() - started
    if attempt > 1:
        for line in lines:
            print(line)

    if any(f.severity == STALE for f in findings):
        print(
            f"\nverify-live: FAILED after {attempt} pass(es) over {elapsed:.1f}s — "
            f"the origin is still serving a previous deploy.",
            file=sys.stderr,
        )
        print(
            "  Propagation is documented as taking seconds. Past the deadline this "
            "is a deploy\n  that did not take, not a race.",
            file=sys.stderr,
        )
        report(findings, sys.stderr)
        return EXIT_WRONG

    print(
        f"\nverify-live: INCONCLUSIVE after {attempt} pass(es) over {elapsed:.1f}s.",
        file=sys.stderr,
    )
    print(
        "  Every finding is consistent with Cloudflare deployment propagation, but "
        "the deploy\n"
        "  did not converge within the deadline, so this check neither confirmed "
        "the bundle nor\n"
        "  caught anything serving the wrong bytes. Treat it as unverified, not as "
        "verified.\n"
        "  Re-running the step is the cheapest next move; if it keeps happening, "
        "compare the\n"
        "  live version's asset manifest rather than raising the deadline.",
        file=sys.stderr,
    )
    report(findings, sys.stderr)
    return EXIT_INCONCLUSIVE


if __name__ == "__main__":
    sys.exit(main(sys.argv))
