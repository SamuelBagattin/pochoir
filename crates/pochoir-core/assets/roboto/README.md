# Roboto, subset

The one font Pochoir ships, as [ADR-0001](../../../../docs/adr/0001-stack-and-dependency-policy.md)
decides: *"We ship **one font that we choose**, in-repo, so the input is not
hostile and the scope collapses."*

| | |
| --- | --- |
| Family | Roboto Regular |
| Upstream | <https://github.com/googlefonts/roboto>, `src/hinted/Roboto-Regular.ttf` |
| Upstream `sha256` | `56a45233d29f11b4dfb86d248e921939d115778f87325e7ae8cc108383d6664d` |
| Upstream size | 515 100 bytes, 3 387 glyphs |
| Licence | **Apache License 2.0** — see [LICENSE](LICENSE), copied from the same repository |
| This file | `Roboto-Regular-subset.ttf`, 21 052 bytes, 347 glyphs |

## Why this font

It had to have TrueType `glyf` outlines rather than CFF, because ADR-0001 scopes
the hand-rolled parser to `glyf` and explicitly excludes PostScript outlines. It
had to cover Latin-1 Supplement and Latin Extended-A, because ADR-0002's mark
text is French. And its licence had to sit comfortably beside crates that are
`MIT OR Apache-2.0`.

**Apache-2.0 is why Roboto rather than a SIL Open Font Licence family.** Several
good candidates — Noto Sans, Liberation Sans, Source Sans — are OFL 1.1, which
is a perfectly free licence but would add a third licence family to a repository
that is already deliberate about them, and whose terms for *derivative* fonts
(and this file is a derivative, being a subset) carry a Reserved Font Name
clause we would have to reason about. Apache-2.0 adds nothing we do not already
have and says nothing about subsets. Given a choice between two free licences
where one is already in the project, the one already in the project wins.

## Why a subset

Unsubsetted, Roboto is 515 100 bytes and three quarters of that is `glyf`, for
three and a half thousand glyphs including Cyrillic and Greek that a French
watermark will never draw. That would have dominated a wasm bundle
[ADR-0003](../../../../docs/adr/0003-pdf-input-rasterisation.md) budgets at
60–120 KiB for the whole core module.

## Reproducing this file

```
make font-subset
```

or directly:

```
scripts/subset-font.py <upstream Roboto-Regular.ttf> \
    crates/pochoir-core/assets/roboto/Roboto-Regular-subset.ttf
```

The script is stdlib-only Python, runs by hand rather than as part of the build,
and nothing at run time depends on it. It keeps `head`, `hhea`, `maxp`, `hmtx`,
`cmap`, `loca` and `glyf` for ASCII printable, Latin-1 Supplement, Latin
Extended-A and a handful of punctuation marks, and drops the hinting bytecode,
which we never interpret because we rasterise with our own scanline filler.

The result was checked against an independent engine before being committed:
macOS CoreText opens it and renders a specimen correctly.

## Modifications

Subsetting as described above. No glyph outline has been altered, no metric
changed and nothing renamed; the file is a strict subset of the upstream tables.
