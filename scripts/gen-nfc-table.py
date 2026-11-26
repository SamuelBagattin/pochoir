#!/usr/bin/env python3
"""Emit the NFC composition table for the characters the bundled font has.

ADR-0002 requires the mark text to be normalised to NFC before layout, and
says that "NFC composition for the Latin-1/Latin Extended-A range we care
about is a small lookup table, not a Unicode library". This writes that table.

The pairs are **derived**, not typed: for every character in the target range,
Python's `unicodedata` is asked for its canonical decomposition, and a pair is
emitted only when recomposing it actually yields the character back. That last
check is what silently handles the Unicode composition exclusions — a character
that NFC does not produce is not emitted — so there is no exclusion list here
to drift out of date.

Stdlib only, run by hand, output pasted into
`crates/pochoir-core/src/mark/nfc.rs` between its marker comments:

    scripts/gen-nfc-table.py

The character set below is the one `scripts/subset-font.py` keeps, minus the
ASCII range, which has no composed characters. Keeping the two scripts' ranges
in step matters: `nfc.rs` has a test asserting that every character this table
can produce is one the bundled font can draw.
"""

import sys
import unicodedata

# Latin-1 Supplement and Latin Extended-A, which is what ADR-0002 names and
# what the font subset covers.
TARGETS = list(range(0x00A0, 0x0180))


def main() -> int:
    pairs = []
    for code in TARGETS:
        character = chr(code)
        decomposition = unicodedata.decomposition(character)
        if not decomposition or decomposition.startswith("<"):
            continue  # No decomposition, or a compatibility one, which NFC keeps.
        parts = decomposition.split()
        if len(parts) != 2:
            continue  # Canonical decompositions in this range are all pairs.
        base, mark = (int(part, 16) for part in parts)
        # The authority on whether this pair composes is NFC itself.
        if unicodedata.normalize("NFC", chr(base) + chr(mark)) != character:
            continue
        pairs.append((base, mark, code))

    pairs.sort()
    marks = sorted({mark for _, mark, _ in pairs})

    print(f"// {len(pairs)} pairs, {len(marks)} combining marks.")
    print("const COMPOSITIONS: [(char, char, char); %d] = [" % len(pairs))
    for base, mark, composed in pairs:
        print(
            "    ('\\u{%04X}', '\\u{%04X}', '\\u{%04X}'), // %s + U+%04X %s"
            % (
                base,
                mark,
                composed,
                chr(composed),
                mark,
                unicodedata.name(chr(mark), "?").removeprefix("COMBINING ").lower(),
            )
        )
    print("];")
    print()
    print("const COMBINING_MARKS: [char; %d] = [" % len(marks))
    for mark in marks:
        print(
            "    '\\u{%04X}', // %s"
            % (mark, unicodedata.name(chr(mark), "?").lower())
        )
    print("];")
    return 0


if __name__ == "__main__":
    sys.exit(main())
