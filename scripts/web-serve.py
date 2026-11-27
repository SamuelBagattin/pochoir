#!/usr/bin/env python3
"""Serve ./dist with the `_headers` rules applied, or explain what they resolve to.

`astro preview` sends none of the production response headers, so the strictest
part of this site — a Content-Security-Policy with no 'unsafe-inline' anywhere —
is invisible locally and first observable on a deploy. That is the wrong order.
This server reads the same `_headers` file that ships inside the assets
directory and applies it, so the policy can be read off a real response, and a
browser pointed at it enforces the real policy.

It reproduces, on purpose, the two Cloudflare behaviours that are easy to get
wrong (ADR-0004 Decision 5):

  * every matching rule applies, in file order, not just the most specific one;
  * a header set twice is comma-joined rather than replaced, unless the second
    rule detaches it first with `! Header-Name`.

So a mistake that would comma-join the Content-Security-Policy in production
comma-joins it here too.

It also implements one piece of the platform proper: `not_found_handling` from
`wrangler.jsonc`. That is a deliberate widening of what this script is, and the
reason is the bug it would otherwise hide. The 404 path is a behaviour we
changed on purpose — `"404-page"` exists so that an unmatched path is answered
by the asset router, where `_headers` applies, rather than by the no-op Worker
wrangler uploads, where it does not. A local server that answered its own bare
404 would show the production headers missing on exactly the path we fixed, and
"documented surprise" is how that gap survived in the first place.

What it still does NOT reproduce: Cloudflare's own default headers (ETag, and
the `Cache-Control` it attaches when `_headers` does not), its asset manifest
lookup, its redirect handling, or anything about a gradual rollout.

**And one thing no local server can settle**, which it therefore says out loud
on every 404 it serves: whether the uploaded no-op Worker defers to the asset
router for an unmatched path, or answers first. Everything here follows from
Cloudflare's documentation and from the artefact wrangler emits; only a real
deploy decides it. `make web-verify-live` asserts it against a live origin.

Usage:
  web-serve.py [--dist DIR] [--port N]        serve
  web-serve.py [--dist DIR] --explain PATH…   print resolved headers and exit

Standard library only. Exit 0 on success, 1 on a problem, 2 on usage.
"""

from __future__ import annotations

import argparse
import functools
import http.server
import json
import socketserver
import sys
from pathlib import Path

CONTROL_FILES = {"_headers", "_redirects", "_routes.json"}

# Sent on every emulated 404, because this is the one thing the local server
# cannot decide and a reader should not have to remember.
NOT_FOUND_CAVEAT = (
    "not_found_handling emulated from wrangler.jsonc; "
    "whether the uploaded no-op Worker defers to the asset router is "
    "unverified and only a live deploy settles it"
)

# Extension -> Content-Type. Deliberately short: these are the types this build
# actually emits, plus wasm, which matters because
# `WebAssembly.instantiateStreaming` rejects anything else.
CONTENT_TYPES = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".json": "application/json; charset=utf-8",
    ".svg": "image/svg+xml",
    ".wasm": "application/wasm",
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".webp": "image/webp",
    ".woff2": "font/woff2",
    ".ico": "image/vnd.microsoft.icon",
    ".txt": "text/plain; charset=utf-8",
    ".map": "application/json; charset=utf-8",
}

Rule = tuple[str, list[tuple[str, str | None]]]


def strip_jsonc(text: str) -> str:
    """Remove `//` and `/* */` comments, respecting string literals.

    wrangler.jsonc is the one place `not_found_handling` is configured, and
    reading it here keeps a single source of truth rather than a second copy
    of the value that can silently disagree with what gets deployed.
    """
    out: list[str] = []
    i, n = 0, len(text)
    in_string = False
    while i < n:
        ch = text[i]
        if in_string:
            out.append(ch)
            if ch == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            out.append(ch)
            i += 1
            continue
        if text.startswith("//", i):
            while i < n and text[i] != "\n":
                i += 1
            continue
        if text.startswith("/*", i):
            end = text.find("*/", i + 2)
            i = n if end == -1 else end + 2
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def read_not_found_handling(repo_root: Path) -> str:
    """`assets.not_found_handling` from wrangler.jsonc, or the platform default."""
    config = repo_root / "web" / "wrangler.jsonc"
    if not config.is_file():
        return "none"
    try:
        parsed = json.loads(strip_jsonc(config.read_text(encoding="utf-8")))
    except (ValueError, OSError):
        return "none"
    value = parsed.get("assets", {}).get("not_found_handling", "none")
    return value if isinstance(value, str) else "none"


def parse_headers_file(path: Path) -> list[Rule]:
    """Parse a Cloudflare `_headers` file into (pattern, directives) rules.

    A directive is (name, value), or (name, None) for a `! Name` detach.
    """
    rules: list[Rule] = []
    current: Rule | None = None

    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if not raw[0].isspace():
            # A path line starts a new rule.
            current = (raw.strip(), [])
            rules.append(current)
            continue
        if current is None:
            raise SystemExit(f"{path}:{lineno}: header line before any path line")
        body = raw.strip()
        if body.startswith("!"):
            name = body[1:].strip()
            if not name:
                raise SystemExit(f"{path}:{lineno}: `!` with no header name")
            current[1].append((name, None))
            continue
        if ":" not in body:
            raise SystemExit(f"{path}:{lineno}: not a header line: {body!r}")
        name, value = body.split(":", 1)
        current[1].append((name.strip(), value.strip()))

    return rules


def matches(pattern: str, url_path: str) -> bool:
    """Cloudflare's trailing-splat matching, which is all this file uses."""
    if pattern.endswith("/*"):
        return url_path.startswith(pattern[:-1])
    if pattern == "/*":
        return True
    if pattern.endswith("*"):
        return url_path.startswith(pattern[:-1])
    return url_path == pattern


def resolve(rules: list[Rule], url_path: str) -> dict[str, str]:
    """Apply every matching rule in file order, comma-joining duplicates."""
    out: dict[str, str] = {}
    for pattern, directives in rules:
        if not matches(pattern, url_path):
            continue
        for name, value in directives:
            key = name.lower()
            if value is None:
                out.pop(key, None)
                continue
            if key in out:
                # Trap 1, reproduced rather than papered over.
                out[key] = f"{out[key]}, {value}"
            else:
                out[key] = value
    # Preserve the canonical spelling of each header name.
    spelled: dict[str, str] = {}
    for pattern, directives in rules:
        if not matches(pattern, url_path):
            continue
        for name, value in directives:
            if value is not None and name.lower() in out:
                spelled[name] = out[name.lower()]
    return spelled


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "pochoir-web-serve"
    protocol_version = "HTTP/1.1"

    def __init__(
        self,
        *args,
        dist: Path,
        rules: list[Rule],
        not_found_handling: str = "none",
        **kwargs,
    ) -> None:
        self.dist = dist
        self.rules = rules
        self.not_found_handling = not_found_handling
        super().__init__(*args, **kwargs)

    def log_message(self, fmt: str, *args) -> None:  # noqa: A003
        sys.stderr.write(f"{self.address_string()} {fmt % args}\n")

    def _resolve_file(self, url_path: str) -> tuple[Path | None, str]:
        """Map a URL path to a file, honouring `auto-trailing-slash`."""
        clean = url_path.split("?", 1)[0].split("#", 1)[0]
        if ".." in clean:
            return None, clean
        rel = clean.lstrip("/")
        candidate = (self.dist / rel).resolve()
        try:
            candidate.relative_to(self.dist.resolve())
        except ValueError:
            return None, clean

        if candidate.is_dir():
            index = candidate / "index.html"
            return (index, clean.rstrip("/") + "/index.html") if index.is_file() else (None, clean)
        if candidate.is_file():
            return candidate, clean
        # /confidentialite -> /confidentialite/index.html
        index = candidate / "index.html"
        if index.is_file():
            return index, clean.rstrip("/") + "/index.html"
        return None, clean

    def do_HEAD(self) -> None:  # noqa: N802
        self._serve(body=False)

    def do_GET(self) -> None:  # noqa: N802
        self._serve(body=True)

    def _serve_not_found(self, url_path: str, body: bool) -> None:
        """Answer an unmatched path the way `not_found_handling` says to.

        With `"404-page"` the asset router answers from `dist/404.html`, so the
        `_headers` rules apply exactly as they do to any other asset — which is
        the entire point of choosing it. With `"none"` they do not, because the
        no-op Worker wrangler uploads generates that response and Cloudflare
        documents that `_headers` is not applied to Worker-generated responses.

        Either way the response carries a header naming what is emulated and
        what is not, so nobody reads a green local 404 as proof about the live
        site. That is the one thing a local server genuinely cannot settle.
        """
        page = self.dist / "404.html"
        clean = url_path.split("?", 1)[0]

        if self.not_found_handling == "404-page" and page.is_file():
            data = page.read_bytes()
            headers = resolve(self.rules, clean)
            content_type = headers.pop("Content-Type", None) or CONTENT_TYPES[".html"]
            self.send_response(404)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header("X-Pochoir-Emulated", NOT_FOUND_CAVEAT)
            self.end_headers()
            if body:
                self.wfile.write(data)
            return

        if self.not_found_handling == "404-page":
            # Configured for a page that is not in the build. In production
            # Cloudflare would fall back to its own 404; here it is worth
            # being loud, because the local server would otherwise look the
            # same as a correctly built one.
            note = (
                b"404\n\nnot_found_handling is \"404-page\" but dist/404.html is "
                b"missing from this build.\n"
            )
        else:
            note = (
                b"404\n\nnot_found_handling is \"none\": Cloudflare answers this "
                b"path from the no-op Worker it\nuploads, and _headers is NOT "
                b"applied to Worker-generated responses. The headers\nmissing "
                b"from this response are missing in production too.\n"
            )

        self.send_response(404)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(note)))
        self.send_header("X-Pochoir-Emulated", NOT_FOUND_CAVEAT)
        self.end_headers()
        if body:
            self.wfile.write(note)

    def _serve(self, body: bool) -> None:
        url_path = self.path
        file_path, _ = self._resolve_file(url_path)

        if file_path is not None and file_path.name in CONTROL_FILES:
            # Cloudflare consumes these; it does not serve them.
            file_path = None

        if file_path is None:
            self._serve_not_found(url_path, body)
            return

        data = file_path.read_bytes()
        headers = resolve(self.rules, url_path.split("?", 1)[0])
        content_type = headers.pop("Content-Type", None) or CONTENT_TYPES.get(
            file_path.suffix.lower(), "application/octet-stream"
        )

        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        for name, value in headers.items():
            self.send_header(name, value)
        self.end_headers()
        if body:
            self.wfile.write(data)


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="web-serve.py")
    parser.add_argument("--dist", type=Path, default=Path("web/dist"))
    parser.add_argument("--port", type=int, default=8788)
    parser.add_argument(
        "--not-found",
        choices=["none", "404-page"],
        default=None,
        help="override assets.not_found_handling; the default reads wrangler.jsonc",
    )
    parser.add_argument(
        "--explain",
        nargs="*",
        default=None,
        metavar="PATH",
        help="print the headers these URL paths resolve to, then exit",
    )
    args = parser.parse_args(argv[1:])

    dist: Path = args.dist
    if not dist.is_dir():
        print(f"web-serve: {dist} is not a directory; build first", file=sys.stderr)
        return 2
    headers_file = dist / "_headers"
    if not headers_file.is_file():
        print(f"web-serve: {headers_file} is missing", file=sys.stderr)
        return 1

    rules = parse_headers_file(headers_file)
    repo_root = Path(__file__).resolve().parent.parent
    not_found = args.not_found or read_not_found_handling(repo_root)

    if args.explain is not None:
        paths = args.explain or ["/", "/_pochoir/app.abcdefgh.js", "/_pochoir/engine/x.wasm"]
        print(f"web-serve: {len(rules)} rule(s) in {headers_file}")
        print(f"web-serve: not_found_handling = {not_found!r}")
        if not_found == "404-page":
            page = dist / "404.html"
            print(
                f"  an unmatched path is answered from dist/404.html "
                f"({'present' if page.is_file() else 'MISSING'}) with the rules "
                f"below for the requested path, status 404"
            )
        else:
            print(
                "  an unmatched path is answered by the no-op Worker wrangler "
                "uploads, which _headers does NOT reach"
            )
        print(f"  unverified: {NOT_FOUND_CAVEAT.split('; ', 1)[1]}")
        for p in paths:
            print(f"\n{p}")
            resolved = resolve(rules, p)
            if not resolved:
                print("  (no headers)")
            for name, value in resolved.items():
                print(f"  {name}: {value}")
        return 0

    handler = functools.partial(
        Handler, dist=dist.resolve(), rules=rules, not_found_handling=not_found
    )
    with Server(("127.0.0.1", args.port), handler) as httpd:
        print(f"web-serve: http://127.0.0.1:{args.port}  ({dist}, {len(rules)} header rule(s))")
        print(f"web-serve: not_found_handling = {not_found!r}")
        print("web-serve: production headers applied; Ctrl-C to stop")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\nweb-serve: stopped")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
