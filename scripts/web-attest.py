#!/usr/bin/env python3
"""Inject Subresource Integrity attributes, then emit the attestation manifest.

ADR-0004 Decision 4, items 3 and 4. Two phases, in this order and not the
other, because injecting SRI changes the HTML bytes:

  1. For every HTML file, compute the SHA-384 of each same-origin subresource
     it references with an SRI-capable element and write an `integrity`
     attribute. Hand-maintained hashes go stale on the first hurried release
     and a stale hash fails closed, which is the one failure mode a user will
     certainly notice.
  2. Hash every file in the output directory and write the manifest: the
     digests, the git commit, the lock-file hashes and the build environment.

The manifest is the deliverable. ADR-0004 Decision 3 publishes it as a GitHub
release asset *and* at a stable path on the site, because only the copy held by
a party that is not our CDN is evidence.

Determinism matters: `files` and `files_digest` depend on the built bytes and
nothing else, so a second build can be compared against the first by comparing
one hex string. The `build` section records the environment and is deliberately
*excluded* from that digest — node's version is informational, not part of what
is served.

Uses only the standard library. Exit 0 on success, 1 on a problem, 2 on usage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from base64 import b64encode
from html.parser import HTMLParser
from pathlib import Path

MANIFEST_VERSION = 1

# Files the platform consumes rather than serves. Cloudflare reads `_headers`
# out of the assets directory and does not expose it as a route; it is hashed
# here anyway, because a change to it changes what a visitor receives.
PLATFORM_CONTROL = {"_headers", "_redirects", "_routes.json"}

# `rel` values for which SRI is specified. SRI-2 adds `integrity` "to the list
# of content attributes for the link and script elements" and to those two
# only; for <link> the destinations are stylesheet, preload and modulepreload.
SRI_LINK_RELS = {"stylesheet", "preload", "modulepreload"}


def sha384_b64(data: bytes) -> str:
    return b64encode(hashlib.sha384(data).digest()).decode("ascii")


def sha384_hex(data: bytes) -> str:
    return hashlib.sha384(data).hexdigest()


# ---------------------------------------------------------------- SRI phase


class SriTargetFinder(HTMLParser):
    """Locates every SRI-capable element and the URL it points at."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        # (line, col, raw_tag_text, url, already_has_integrity)
        self.targets: list[tuple[int, int, str, str, bool]] = []

    def _consider(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        lowered = {name.lower(): (value or "") for name, value in attrs}
        url: str | None = None

        if tag == "script":
            url = lowered.get("src")
        elif tag == "link":
            rels = {r.lower() for r in lowered.get("rel", "").split()}
            if rels & SRI_LINK_RELS:
                url = lowered.get("href")

        if not url:
            return

        raw = self.get_starttag_text() or ""
        line, col = self.getpos()
        self.targets.append((line, col, raw, url, "integrity" in lowered))

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self._consider(tag.lower(), attrs)

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self._consider(tag.lower(), attrs)


def _line_offsets(text: str) -> list[int]:
    """Byte-free character offset of the start of each 1-indexed line."""
    offsets = [0, 0]
    for line in text.split("\n")[:-1]:
        offsets.append(offsets[-1] + len(line) + 1)
    return offsets


def inject_sri(dist: Path, verbose: bool = True) -> tuple[int, int]:
    """Write `integrity` on every same-origin SRI-capable reference.

    Returns (elements injected, elements already carrying integrity).
    """
    injected = 0
    already = 0

    for html_path in sorted(dist.rglob("*.html")):
        text = html_path.read_text(encoding="utf-8")
        finder = SriTargetFinder()
        finder.feed(text)
        finder.close()

        offsets = _line_offsets(text)
        # Splice from the end so earlier offsets stay valid.
        edits: list[tuple[int, int, str]] = []

        for line, col, raw, url, has_integrity in finder.targets:
            if has_integrity:
                already += 1
                continue
            # Same-origin, root-relative references only. Anything else is
            # either cross-origin — which the CSP forbids anyway — or a data:
            # URI, which `assetsInlineLimit: 0` is set to prevent.
            if not url.startswith("/") or url.startswith("//"):
                if verbose:
                    print(
                        f"  skip {url!r} in {html_path.relative_to(dist)}: not root-relative",
                        file=sys.stderr,
                    )
                continue
            target = dist / url.lstrip("/").split("?", 1)[0].split("#", 1)[0]
            if not target.is_file():
                raise SystemExit(
                    f"web-attest: {html_path.relative_to(dist)} references {url} "
                    f"which is not in the build output ({target})"
                )

            start = offsets[line] + col
            if not text.startswith(raw, start):
                raise SystemExit(
                    f"web-attest: could not locate {raw[:60]!r} at "
                    f"{html_path.relative_to(dist)}:{line}:{col}"
                )
            digest = sha384_b64(target.read_bytes())
            # Insert before the closing bracket, keeping any self-closing slash.
            body = raw[:-1].rstrip()
            trailing = ""
            if body.endswith("/"):
                body = body[:-1].rstrip()
                trailing = " /"
            new = f'{body} integrity="sha384-{digest}"{trailing}>'
            edits.append((start, start + len(raw), new))
            injected += 1

        if edits:
            for start, end, new in sorted(edits, reverse=True):
                text = text[:start] + new + text[end:]
            html_path.write_text(text, encoding="utf-8")
            if verbose:
                print(f"  {html_path.relative_to(dist)}: {len(edits)} integrity attribute(s)")

    return injected, already



# ------------------------------------------------------- module graph

# Rollup emits import specifiers in a small number of shapes, all with the
# specifier as a quoted relative path. This matches those shapes and nothing
# else; a specifier it misses shows up in the completeness check below rather
# than silently going unattested.
_IMPORT_RE = re.compile(
    r"""(?:\bfrom|\bimport)\s*\(?\s*(['"])(\.{1,2}/[^'"]+)\1""",
)


def module_imports(path: Path) -> list[str]:
    """Relative specifiers statically or dynamically imported by one module."""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    return [m.group(2) for m in _IMPORT_RE.finditer(text)]


def module_closure(dist: Path, entry: Path) -> list[Path]:
    """Every module `entry` reaches, transitively, excluding `entry` itself."""
    seen: set[Path] = {entry.resolve()}
    order: list[Path] = []
    queue = [entry]
    while queue:
        current = queue.pop(0)
        for spec in module_imports(current):
            target = (current.parent / spec).resolve()
            if target in seen or not target.is_file():
                continue
            try:
                target.relative_to(dist.resolve())
            except ValueError:
                continue
            seen.add(target)
            order.append(target)
            queue.append(target)
    return sorted(order)


def inject_modulepreload(dist: Path, verbose: bool = True) -> int:
    """Emit `<link rel="modulepreload" integrity>` for every imported chunk.

    MEASURED, not assumed: an `integrity` attribute on a `<script type=module>`
    covers that file's own bytes and nothing it imports. With a byte appended to
    the chunk the entry statically imports, the injected code *ran*, the page
    mounted and no error was reported. With a `modulepreload` link carrying the
    chunk's hash, the same tamper is blocked and the page does not mount.

    It is also what makes `Integrity-Policy: blocked-destinations=(script)`
    shippable. Without these links that header blocks the import outright — the
    chunk request fails and the application is blank — because an ES module
    import has no way to carry integrity metadata of its own.
    """
    added = 0
    for html_path in sorted(dist.rglob("*.html")):
        text = html_path.read_text(encoding="utf-8")
        finder = SriTargetFinder()
        finder.feed(text)
        finder.close()

        existing = {url for _, _, raw, url, _ in finder.targets if "modulepreload" in raw}
        entries = [
            dist / url.lstrip("/")
            for _, _, raw, url, _ in finder.targets
            if raw.lower().startswith("<script") and url.startswith("/")
        ]
        if not entries:
            continue

        # The insertion point is immediately before the first <script>, so the
        # parser sees every preload before it needs the module.
        anchor = None
        for line, col, raw, _url, _has in finder.targets:
            if raw.lower().startswith("<script"):
                anchor = _line_offsets(text)[line] + col
                break
        if anchor is None:
            continue

        links: list[str] = []
        for entry in entries:
            if not entry.is_file():
                continue
            for module in module_closure(dist, entry):
                href = "/" + module.relative_to(dist.resolve()).as_posix()
                if href in existing:
                    continue
                existing.add(href)
                digest = sha384_b64(module.read_bytes())
                links.append(
                    f'<link rel="modulepreload" href="{href}" '
                    f'integrity="sha384-{digest}">'
                )

        if links:
            text = text[:anchor] + "".join(links) + text[anchor:]
            html_path.write_text(text, encoding="utf-8")
            added += len(links)
            if verbose:
                print(f"  {html_path.relative_to(dist)}: {len(links)} modulepreload link(s)")

    return added


def report_uncovered(dist: Path, assets_dir: str = "_pochoir") -> list[str]:
    """JavaScript in the build that no integrity mechanism can cover.

    Two kinds end up here and both are honest limitations rather than bugs:
    a dedicated worker script, because `new Worker(url)` has no integrity
    option and its fetch destination is `worker` rather than `script`; and a
    chunk nothing references, which is dead weight in the bundle.
    """
    referenced: set[Path] = set()
    for html_path in dist.rglob("*.html"):
        finder = SriTargetFinder()
        finder.feed(html_path.read_text(encoding="utf-8"))
        finder.close()
        for _line, _col, _raw, url, _has in finder.targets:
            if not url.startswith("/"):
                continue
            target = dist / url.lstrip("/")
            if target.is_file():
                referenced.add(target.resolve())
                referenced.update(module_closure(dist, target))

    uncovered = []
    asset_root = dist / assets_dir
    candidates = sorted(asset_root.glob("**/*.js")) if asset_root.is_dir() else []
    for js in candidates:
        if js.resolve() not in referenced:
            uncovered.append(js.relative_to(dist).as_posix())
    return uncovered


# ----------------------------------------------------------- manifest phase


def _git(repo: Path, *args: str) -> str | None:
    try:
        out = subprocess.run(
            ["git", *args],
            cwd=repo,
            capture_output=True,
            text=True,
            check=True,
        )
        return out.stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def _tool_version(*cmd: str) -> str | None:
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return out.stdout.strip().splitlines()[0] if out.stdout.strip() else None
    except (OSError, subprocess.CalledProcessError, IndexError):
        return None


def _file_hash(path: Path) -> str | None:
    return sha384_hex(path.read_bytes()) if path.is_file() else None


ENGINE_DIR = "_pochoir/engine"


def find_engine(dist: Path) -> Path | None:
    """The compiled wasm engine, if `make web-engine` has placed one.

    It lives in its own directory so that `_headers` can set
    `Content-Type: application/wasm` with a trailing splat, which is the only
    pattern form Cloudflare documents.
    """
    engine_dir = dist / ENGINE_DIR
    if not engine_dir.is_dir():
        return None
    modules = sorted(engine_dir.glob("*.wasm"))
    if len(modules) > 1:
        raise SystemExit(
            f"web-attest: {len(modules)} wasm modules in {ENGINE_DIR}; expected one. "
            "A stale module left beside a new one would be served to anyone "
            "holding an old HTML. Clean the build."
        )
    return modules[0] if modules else None


def build_manifest(repo: Path, dist: Path, engine: Path | None) -> dict:
    files: dict[str, dict[str, object]] = {}
    for path in sorted(p for p in dist.rglob("*") if p.is_file()):
        rel = path.relative_to(dist).as_posix()
        data = path.read_bytes()
        files[rel] = {
            "sha384": sha384_hex(data),
            "integrity": f"sha384-{sha384_b64(data)}",
            "bytes": len(data),
        }

    # One hex string standing for the whole output. The canonical form is
    # "path\0sha384\n" per file, sorted by path — stable across platforms and
    # independent of JSON formatting.
    canonical = "".join(
        f"{path}\0{meta['sha384']}\n" for path, meta in sorted(files.items())
    ).encode("utf-8")

    commit = _git(repo, "rev-parse", "HEAD")
    status = _git(repo, "status", "--porcelain")

    manifest: dict[str, object] = {
        "manifest_version": MANIFEST_VERSION,
        "project": "pochoir",
        # Reproducible section: derived from the built bytes and nothing else.
        "files_digest": sha384_hex(canonical),
        "files": files,
        "platform_control": sorted(PLATFORM_CONTROL & set(files)),
        "source": {
            "commit": commit,
            "clean": (status == "") if status is not None else None,
            "pnpm_lock_sha384": _file_hash(repo / "web" / "pnpm-lock.yaml"),
            "cargo_lock_sha384": _file_hash(repo / "Cargo.lock"),
        },
        # Informational, excluded from files_digest on purpose.
        "build": {
            "node": _tool_version("node", "--version"),
            "pnpm": _tool_version("pnpm", "--version"),
            "rustc": _tool_version("rustc", "--version"),
            "rustflags": None,
        },
        # ADR-0004 Decisions 1 and 2. Filled in when crates/pochoir-wasm
        # produces a module; see docs/DEPLOY.md for the exact wiring.
        "engine": None,
    }

    if engine is not None:
        if not engine.is_file():
            raise SystemExit(f"web-attest: engine artefact {engine} does not exist")
        data = engine.read_bytes()
        manifest["engine"] = {
            # The served path, not a local one: this is what verify-live.py
            # fetches from the production origin.
            "path": engine.relative_to(dist).as_posix(),
            "sha384": sha384_hex(data),
            "integrity": f"sha384-{sha384_b64(data)}",
            "bytes": len(data),
        }

    return manifest



# ------------------------------------------------------------- verify mode

# JavaScript that is allowed to be unattested, with the reason. Anything else
# appearing here is a new hole, and `--verify` fails on it.
UNCOVERED_ALLOWED = (
    (
        "worker-",
        "loaded by `new Worker(url)`, which has no integrity option; its fetch "
        "destination is `worker`, not `script`, so Integrity-Policy does not "
        "reach it either. Only the manifest and a rebuild can detect a swap here",
    ),
    (
        "client.",
        "Astro's island bootstrap. Referenced by nothing, because the island is "
        "hand-mounted; it is emitted and never loaded",
    ),
)


def _allowed_uncovered(rel: str) -> str | None:
    name = rel.rsplit("/", 1)[-1]
    for prefix, reason in UNCOVERED_ALLOWED:
        if name.startswith(prefix):
            return reason
    return None


def verify(dist: Path) -> int:
    """Assert the attestation is present, correct and complete. Mutates nothing.

    Three things are checked, and the third is the one that matters: a chunk
    the import scanner failed to see would otherwise reach production with no
    integrity metadata, where `Integrity-Policy: blocked-destinations=(script)`
    makes the page blank. Here it is a build failure instead.
    """
    problems: list[str] = []
    checked = 0
    preloaded_total = 0

    for html_path in sorted(dist.rglob("*.html")):
        rel_html = html_path.relative_to(dist).as_posix()
        finder = SriTargetFinder()
        finder.feed(html_path.read_text(encoding="utf-8"))
        finder.close()

        preloaded: set[str] = set()
        entries: list[Path] = []

        for _line, _col, raw, url, has_integrity in finder.targets:
            lowered = raw.lower()
            is_icon = 'rel="icon"' in lowered or "rel='icon'" in lowered
            if is_icon:
                continue
            if not url.startswith("/"):
                continue
            target = dist / url.lstrip("/")
            if not target.is_file():
                problems.append(f"{rel_html}: references {url}, which is not in the build")
                continue
            checked += 1
            if not has_integrity:
                problems.append(f"{rel_html}: {url} carries no integrity attribute")
                continue
            # The attribute must match the bytes on disk, not merely exist.
            want = f"sha384-{sha384_b64(target.read_bytes())}"
            if want not in raw:
                problems.append(
                    f"{rel_html}: {url} integrity attribute does not match the file "
                    f"(expected {want})"
                )
            if "modulepreload" in lowered:
                preloaded.add(url)
                preloaded_total += 1
            if lowered.startswith("<script"):
                entries.append(target)

        for entry in entries:
            for module in module_closure(dist, entry):
                href = "/" + module.relative_to(dist.resolve()).as_posix()
                if href not in preloaded:
                    problems.append(
                        f"{rel_html}: {href} is imported by "
                        f"/{entry.relative_to(dist).as_posix()} but has no "
                        f"modulepreload link, so its bytes are unattested"
                    )

    for rel in report_uncovered(dist):
        if _allowed_uncovered(rel) is None:
            problems.append(
                f"{rel}: JavaScript in the build that no integrity mechanism covers "
                f"and that is not a known exception"
            )

    manifest_path = dist / "manifest.json"
    if not manifest_path.is_file():
        problems.append("manifest.json is missing from the build output")
    else:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        for rel, meta in manifest.get("files", {}).items():
            path = dist / rel
            if not path.is_file():
                problems.append(f"manifest lists {rel}, which is not in the build")
            elif sha384_hex(path.read_bytes()) != meta["sha384"]:
                problems.append(f"manifest hash for {rel} does not match the file")

    print(f"web-attest --verify: {checked} integrity attribute(s) checked, "
          f"{preloaded_total} modulepreload link(s)")
    for rel in report_uncovered(dist):
        reason = _allowed_uncovered(rel)
        if reason is not None:
            print(f"  unattested, by nature: {rel}\n      {reason}")

    if problems:
        print("\nweb-attest --verify: FAILED", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print("web-attest --verify: attestation complete and matching.")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="web-attest.py",
        description="Inject SRI attributes and emit the ADR-0004 manifest.",
    )
    parser.add_argument("dist", nargs="?", default="web/dist", type=Path)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=None,
        help="where to write the manifest (default: <dist>/manifest.json)",
    )
    parser.add_argument("--no-sri", action="store_true", help="manifest only")
    parser.add_argument(
        "--verify",
        action="store_true",
        help="assert the attestation is present, correct and complete; mutate nothing",
    )
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv[1:])

    dist: Path = args.dist
    if not dist.is_dir():
        print(f"web-attest: {dist} is not a directory; build first", file=sys.stderr)
        return 2

    repo = Path(__file__).resolve().parent.parent
    verbose = not args.quiet

    if args.verify:
        return verify(dist)

    if not args.no_sri:
        if verbose:
            print("web-attest: subresource integrity")
        injected, already = inject_sri(dist, verbose=verbose)
        if verbose:
            print(f"  {injected} injected, {already} already present")
            print("web-attest: module preloads")
        preloads = inject_modulepreload(dist, verbose=verbose)
        if verbose:
            print(f"  {preloads} modulepreload link(s) added")
        uncovered = report_uncovered(dist)
        if uncovered and verbose:
            print("web-attest: JavaScript no integrity mechanism covers:")
            for path in uncovered:
                print(f"  {path}")

    manifest_path = args.manifest or (dist / "manifest.json")
    # The manifest describes the files it sits beside; it must not describe
    # itself, so it is written after the inventory and excluded from it.
    if manifest_path.exists() and manifest_path.is_relative_to(dist):
        manifest_path.unlink()

    manifest = build_manifest(repo, dist, find_engine(dist))
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=False, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    if verbose:
        print(f"web-attest: {len(manifest['files'])} file(s) -> {manifest_path}")
        print(f"web-attest: files_digest {manifest['files_digest']}")
        if manifest["engine"] is None:
            print("web-attest: engine: none yet (crates/pochoir-wasm has not landed)")
    else:
        print(manifest["files_digest"])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
