#!/usr/bin/env python3
"""Place the compiled wasm engine at a content-addressed path, and point the
bundle at it by rewriting two lines of `web/src/engine/engine-manifest.ts`.

This runs BEFORE `astro build`, not after, which is the whole reason it is a
separate script. The module goes into `web/public/`, so Astro copies it into
`dist/` verbatim; the two constants are TypeScript the build then compiles
into the hashed bundle. Doing it afterwards would mean the hash the page
carries was not in the page the hash was computed over.

`engine-manifest.ts` is a committed source file whose doc comment is the
contract — read it, not this header, for what the values mean. Only its two
`export const` lines are rewritten, so the contract travels with the file in
both directions and a developer's diff while an engine is placed is exactly
two lines. `--clear` puts the two fallback values back, which is how someone
returns to `pnpm dev` after a production build without a broken checkout.

The module is placed under `_pochoir/engine/` rather than beside the other
assets so that `_headers` can set `Content-Type: application/wasm` with a
trailing splat — the only pattern form Cloudflare documents, and requirement 3
of the contract. `WebAssembly.instantiateStreaming` rejects anything else.

Usage:
  scripts/web-engine.py target/wasm32-unknown-unknown/release/pochoir_wasm.wasm
  scripts/web-engine.py --clear          remove it and restore the fallback

Standard library only. Exit 0 on success, 2 on usage or a bad artefact.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import sys
from base64 import b64encode
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ENGINE_DIR = REPO / "web" / "public" / "_pochoir" / "engine"
MANIFEST_TS = REPO / "web" / "src" / "engine" / "engine-manifest.ts"

# The committed development fallback, as `engine-manifest.ts` documents it: an
# unhashed path for an artefact the developer drops into web/public/ by hand,
# and no integrity, because inventing one for a local file would be a lie.
FALLBACK_URL = "/pochoir_wasm.wasm"
FALLBACK_INTEGRITY = "undefined"

# Vite's own asset hashes are 8 characters; this is 16.
HASH_CHARS = 16

_URL_RE = re.compile(
    r"^(export const ENGINE_URL: string = )'[^']*'(;)$", re.MULTILINE
)
_INTEGRITY_RE = re.compile(
    r"^(export const ENGINE_INTEGRITY: string \| undefined = ).*?(;)$", re.MULTILINE
)


def rewrite(url_literal: str, integrity_literal: str) -> None:
    """Replace the two export values, leaving the contract comment untouched."""
    if not MANIFEST_TS.is_file():
        raise SystemExit(
            f"web-engine: {MANIFEST_TS.relative_to(REPO)} is missing. It is a "
            f"committed source file, not a generated one — restore it with "
            f"`git checkout -- {MANIFEST_TS.relative_to(REPO)}`."
        )
    text = MANIFEST_TS.read_text(encoding="utf-8")
    new, url_hits = _URL_RE.subn(rf"\g<1>{url_literal}\g<2>", text)
    new, integrity_hits = _INTEGRITY_RE.subn(
        rf"\g<1>{integrity_literal}\g<2>", new
    )
    if url_hits != 1 or integrity_hits != 1:
        raise SystemExit(
            f"web-engine: expected exactly one ENGINE_URL and one "
            f"ENGINE_INTEGRITY declaration in "
            f"{MANIFEST_TS.relative_to(REPO)}, found {url_hits} and "
            f"{integrity_hits}. The file's shape changed; update this script "
            f"rather than hand-editing the values."
        )
    MANIFEST_TS.write_text(new, encoding="utf-8")


def clear() -> int:
    if ENGINE_DIR.is_dir():
        shutil.rmtree(ENGINE_DIR)
        print(f"web-engine: removed {ENGINE_DIR.relative_to(REPO)}")
    rewrite(f"'{FALLBACK_URL}'", FALLBACK_INTEGRITY)
    print(
        f"web-engine: {MANIFEST_TS.relative_to(REPO)} back to the development "
        f"fallback ({FALLBACK_URL}, no integrity)"
    )
    print(
        "web-engine: for `pnpm dev`, drop the artefact at "
        "web/public/pochoir_wasm.wasm (uncommitted)"
    )
    return 0


def place(source: Path) -> int:
    if not source.is_file():
        print(f"web-engine: {source} does not exist", file=sys.stderr)
        return 2
    data = source.read_bytes()
    if data[:4] != b"\x00asm":
        print(
            f"web-engine: {source} does not start with the WebAssembly magic "
            f"(got {data[:4]!r}); refusing to place it",
            file=sys.stderr,
        )
        return 2

    # Named after the artefact, not after a crate name hard-coded here: the
    # engine crate was `pochoir-core` once and is `pochoir-wasm` now, and the
    # served path should follow the file.
    short = hashlib.sha256(data).hexdigest()[:HASH_CHARS]
    name = f"{source.stem}.{short}.wasm"

    # One module at a time. A stale module left beside a new one would still be
    # served to anyone holding an old HTML — which is the point of content
    # addressing — but it would also be a module nothing attests any more.
    if ENGINE_DIR.is_dir():
        shutil.rmtree(ENGINE_DIR)
    ENGINE_DIR.mkdir(parents=True)
    (ENGINE_DIR / name).write_bytes(data)

    url = f"/_pochoir/engine/{name}"
    integrity = f"sha384-{b64encode(hashlib.sha384(data).digest()).decode('ascii')}"
    rewrite(f"'{url}'", f"'{integrity}'")

    print(f"web-engine: {len(data)} bytes -> web/public{url}")
    print(f"web-engine: {integrity}")
    print(f"web-engine: rewrote ENGINE_URL and ENGINE_INTEGRITY in "
          f"{MANIFEST_TS.relative_to(REPO)}")
    print("web-engine: `make web-engine ENGINE=--clear` restores the fallback")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="web-engine.py")
    parser.add_argument("engine", nargs="?", type=Path)
    parser.add_argument("--clear", action="store_true")
    args = parser.parse_args(argv[1:])

    if args.clear:
        return clear()
    if args.engine is None:
        parser.print_usage(sys.stderr)
        return 2
    return place(args.engine)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
