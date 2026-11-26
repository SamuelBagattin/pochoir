#!/usr/bin/env python3
"""Fetch what an origin is actually serving and compare it with the manifest.

ADR-0004 Decision 4 item 6. This is the only check in the whole scheme that can
catch adversary #2 — a deploy that did not come from CI — because every other
step reasons about bytes we produced rather than bytes a visitor receives. It
is also the step most likely to be skipped, being the only one that touches the
network after a green build, which is why it is a Makefile target and a
workflow step rather than a note in a runbook.

It should run on a schedule as well as at release. A bundle swapped a week
after deploy is exactly the attack a release-time check cannot see.

What it checks, per file in the manifest:
  * the response status;
  * the SHA-384 of the body against the manifest;
  * for the engine module, that Content-Type is exactly `application/wasm`,
    because `WebAssembly.instantiateStreaming` rejects anything else.

And once, for the site root: that the security headers the deploy is supposed
to be carrying are present, since `_headers` is uploaded as part of the bundle
and a silently dropped header is indistinguishable from a working deploy until
someone reads a response.

Standard library only — urllib, not a client we would have to justify.
Exit 0 when everything matches, 1 on any mismatch, 2 on usage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
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


def fetch(url: str, timeout: float) -> tuple[int, bytes, dict[str, str]]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            headers = {k.lower(): v for k, v in response.headers.items()}
            return response.status, response.read(), headers
    except urllib.error.HTTPError as exc:
        headers = {k.lower(): v for k, v in exc.headers.items()} if exc.headers else {}
        return exc.code, exc.read(), headers


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="verify-live.py")
    parser.add_argument("origin", help="https://pochoir.app, or a preview URL")
    parser.add_argument("--manifest", type=Path, default=Path("web/dist/manifest.json"))
    parser.add_argument("--timeout", type=float, default=20.0)
    args = parser.parse_args(argv[1:])

    origin = args.origin.rstrip("/")
    if not origin:
        print("verify-live: no origin given", file=sys.stderr)
        return 2
    if not args.manifest.is_file():
        print(f"verify-live: {args.manifest} is missing; build first", file=sys.stderr)
        return 2

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    problems: list[str] = []
    checked = 0

    print(f"verify-live: {origin}")
    print(f"  manifest files_digest {manifest['files_digest']}")

    for rel, meta in sorted(manifest.get("files", {}).items()):
        url = f"{origin}/{rel}"
        status, body, headers = fetch(url, args.timeout)

        if rel in NOT_SERVED:
            if status == 200:
                problems.append(f"{rel}: served with 200; it should not be reachable")
            else:
                print(f"  {status:>3}  {rel}  (not served, as expected)")
            continue

        checked += 1
        if status != 200:
            problems.append(f"{rel}: HTTP {status}")
            print(f"  {status:>3}  {rel}")
            continue

        got = hashlib.sha384(body).hexdigest()
        if got != meta["sha384"]:
            problems.append(
                f"{rel}: body does not match the manifest\n"
                f"      manifest {meta['sha384']}\n"
                f"      served   {got}"
            )
            print(f"  200  {rel}  MISMATCH")
        else:
            print(f"  200  {rel}  ok ({len(body)} bytes)")

        if rel.endswith(".wasm"):
            ctype = headers.get("content-type", "")
            if ctype.split(";")[0].strip() != "application/wasm":
                problems.append(
                    f"{rel}: Content-Type is {ctype!r}; "
                    f"WebAssembly.instantiateStreaming requires application/wasm"
                )

    engine = manifest.get("engine")
    if engine:
        url = f"{origin}/{engine['path']}" if "path" in engine else None
        if url:
            status, body, headers = fetch(url, args.timeout)
            checked += 1
            if status != 200:
                problems.append(f"{engine['path']}: HTTP {status}")
            elif hashlib.sha384(body).hexdigest() != engine["sha384"]:
                problems.append(f"{engine['path']}: engine bytes do not match the manifest")
            elif headers.get("content-type", "").split(";")[0].strip() != "application/wasm":
                problems.append(
                    f"{engine['path']}: Content-Type is "
                    f"{headers.get('content-type')!r}, not application/wasm"
                )

    status, _body, headers = fetch(f"{origin}/", args.timeout)
    print("  response headers on /:")
    for name in REQUIRED_HEADERS:
        value = headers.get(name)
        if value is None:
            problems.append(f"/: {name} is absent from the response")
            print(f"    {name}: ABSENT")
        else:
            print(f"    {name}: {value[:110]}{'…' if len(value) > 110 else ''}")
    csp = headers.get("content-security-policy", "")
    if "'unsafe-inline'" in csp or "'unsafe-eval'" in csp:
        problems.append("/: the served CSP contains 'unsafe-inline' or 'unsafe-eval'")

    if problems:
        print(f"\nverify-live: FAILED after {checked} file(s)", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(f"\nverify-live: {checked} file(s) served, every byte matches the manifest.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
