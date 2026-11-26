#!/usr/bin/env python3
"""Fail if the built site contains anything that would need CSP 'unsafe-inline'.

This is not style policing. The deployed Content-Security-Policy
(web/public/_headers, ADR-0004 Decision 5) carries no 'unsafe-inline' in either
`script-src` or `style-src`, so every one of the constructs below would be
*blocked in production* while working perfectly in `astro dev` and in
`astro preview`, neither of which sends the header.

The specific trap this exists to catch: the Svelte island in
`src/pages/index.astro` is mounted by hand rather than with a `client:*`
directive, because Astro's island machinery emits an `<astro-island>` custom
element, an inline bootstrap `<script>` and an inline `display: contents`
`<style>`. Adding `client:load` is a one-word change that blanks the deployed
page and breaks nothing a developer would notice locally.

Measured on the tree this was written against: zero of all five findings.

Uses only the standard library: html.parser, per the repository's dependency
policy. Exit status 0 when clean, 1 when not, 2 on a usage error.
"""

from __future__ import annotations

import sys
from html.parser import HTMLParser
from pathlib import Path

# Attributes that run script from markup. CSP's script-src blocks every one of
# them without 'unsafe-inline'; there is no `'unsafe-hashes'` in our policy
# either, which would be the only other way to permit them.
_EVENT_ATTR_PREFIX = "on"


class InlineFinder(HTMLParser):
    """Collects every construct in one HTML file that needs 'unsafe-inline'."""

    def __init__(self, path: Path) -> None:
        # convert_charrefs stays on; we never read text content as data.
        super().__init__(convert_charrefs=True)
        self.path = path
        self.findings: list[tuple[str, int, str]] = []
        # Set while inside a <script> with no src, so its body can be judged
        # non-empty before it is reported. An empty <script></script> is
        # harmless and the build has never emitted one, but reporting it would
        # be a false positive.
        self._pending_inline_script: tuple[int, str] | None = None

    # -- helpers ----------------------------------------------------------

    def _report(self, kind: str, note: str) -> None:
        line, col = self.getpos()
        self.findings.append((kind, line, f"{note} (line {line}, col {col})"))

    def _scan_attrs(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        names = {name.lower() for name, _ in attrs}

        if "style" in names:
            self._report("style-attribute", f"<{tag} style=…>")

        for name, _ in attrs:
            lowered = name.lower()
            # `on` alone is not an event handler; `online` is not either, but
            # no HTML attribute starts with `on` and is not a handler, so the
            # prefix test is the right one and errs towards reporting.
            if lowered.startswith(_EVENT_ATTR_PREFIX) and len(lowered) > 2:
                self._report("event-handler-attribute", f"<{tag} {lowered}=…>")

    # -- parser hooks -----------------------------------------------------

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        tag = tag.lower()
        self._scan_attrs(tag, attrs)

        if tag == "style":
            self._report("inline-style-element", "<style> element")

        if tag == "script":
            has_src = any(name.lower() == "src" for name, _ in attrs)
            if not has_src:
                line, col = self.getpos()
                self._pending_inline_script = (line, col)

        # The tell of a `client:*` directive having been added to a component.
        if tag == "astro-island":
            self._report(
                "astro-island",
                "<astro-island>: a client:* directive was added somewhere; "
                "mount the island by hand instead",
            )

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        # Self-closing form never has a body, so an inline script cannot hide
        # here; only the attribute checks apply.
        tag = tag.lower()
        self._scan_attrs(tag, attrs)
        if tag == "astro-island":
            self._report("astro-island", "<astro-island/>: a client:* directive was added")

    def handle_data(self, data: str) -> None:
        if self._pending_inline_script is not None and data.strip():
            line, col = self._pending_inline_script
            kind = "inline-script"
            self.findings.append(
                (
                    kind,
                    line,
                    f"<script> with no src and {len(data.strip())} bytes of body "
                    f"(line {line}, col {col})",
                )
            )
            self._pending_inline_script = None

    def handle_endtag(self, tag: str) -> None:
        if tag.lower() == "script":
            self._pending_inline_script = None


def check(root: Path) -> int:
    html_files = sorted(root.rglob("*.html"))
    if not html_files:
        print(f"check-web-inline: no HTML under {root}; build first", file=sys.stderr)
        return 2

    counts: dict[str, int] = {
        "inline-script": 0,
        "inline-style-element": 0,
        "style-attribute": 0,
        "event-handler-attribute": 0,
        "astro-island": 0,
    }
    failed = False

    for path in html_files:
        finder = InlineFinder(path)
        finder.feed(path.read_text(encoding="utf-8"))
        finder.close()
        for kind, _line, note in finder.findings:
            counts[kind] = counts.get(kind, 0) + 1
            failed = True
            rel = path.relative_to(root)
            print(f"{rel}: {kind}: {note}", file=sys.stderr)

    print(f"check-web-inline: {len(html_files)} HTML file(s) under {root}")
    for kind in sorted(counts):
        print(f"  {counts[kind]:>3}  {kind}")

    if failed:
        print(
            "\ncheck-web-inline: FAILED. The deployed CSP has no 'unsafe-inline';\n"
            "every finding above is blocked in production and works in dev.\n"
            "See web/public/_headers and docs/adr/0004-*.md Decision 5.",
            file=sys.stderr,
        )
        return 1

    print("check-web-inline: clean — nothing here needs 'unsafe-inline'.")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print("usage: check-web-inline.py [dist-directory]", file=sys.stderr)
        return 2
    root = Path(argv[1]) if len(argv) == 2 else Path("web/dist")
    if not root.is_dir():
        print(f"check-web-inline: {root} is not a directory; build first", file=sys.stderr)
        return 2
    return check(root)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
