#!/usr/bin/env python3
"""Cut a TrueType font down to the glyphs Pochoir's watermark can draw.

ADR-0001 decides that we ship **one font we choose, in-repo**, and hand-roll
the parsing, on the grounds that a font we control is not hostile input and the
scope therefore collapses. The same reasoning applies to its size: we need a
few hundred Latin glyphs, and Roboto ships three and a half thousand with
Cyrillic, Greek, hinting bytecode, layout tables and glyph names. Unsubsetted it
is 515 100 bytes, three quarters of which is `glyf`, which would dominate a wasm
bundle ADR-0003 budgets at 60-120 KiB.

So this writes a new font containing only:

    head  hhea  maxp  hmtx  cmap  loca  glyf

for the character set below, with TrueType hinting instructions dropped (we
rasterise with our own scanline filler and never interpret them). Everything
else - GPOS, GSUB, GDEF, post, name, OS/2, hdmx, LTSH, gasp, fpgm, prep, cvt -
goes, because nothing in our pipeline reads it.

This is a **build-time tool**, run by hand and its output committed. It is not
part of `make check` and nothing at run time depends on Python. It exists so
that the committed asset is reproducible from the upstream file rather than
being a binary somebody has to take on trust:

    make font-subset

Usage:
    scripts/subset-font.py <input.ttf> <output.ttf>

Stdlib only, deliberately: adding fonttools to run this would be a dependency
on the critical path of auditing our own asset.
"""

import struct
import sys

# --- The character set -------------------------------------------------------
#
# ASCII printable, Latin-1 Supplement and Latin Extended-A, which is the range
# ADR-0002's NFC decision names: "We need only Latin-1 Supplement and Latin
# Extended-A". Plus the handful of punctuation marks a French mark text
# realistically contains and that are not in those blocks.

WANTED = (
    list(range(0x0020, 0x007F))  # ASCII printable
    + list(range(0x00A0, 0x0100))  # Latin-1 Supplement
    + list(range(0x0100, 0x0180))  # Latin Extended-A
    + [
        0x2013,  # en dash
        0x2014,  # em dash
        0x2018,  # left single quote
        0x2019,  # right single quote, the French apostrophe
        0x201C,  # left double quote
        0x201D,  # right double quote
        0x2026,  # horizontal ellipsis
        0x20AC,  # euro sign
        0x2192,  # rightwards arrow
    ]
)

KEEP_TABLES = [b"cmap", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp"]


def read_tables(data):
    """The sfnt table directory, as {tag: (offset, length)}."""
    version, count = struct.unpack(">IH", data[:6])
    if version not in (0x00010000, 0x74727565):
        raise SystemExit(f"not a TrueType outline font: sfnt version {version:#010x}")
    tables = {}
    for i in range(count):
        at = 12 + i * 16
        tag = data[at : at + 4]
        _checksum, offset, length = struct.unpack(">III", data[at + 4 : at + 16])
        tables[tag] = (offset, length)
    return tables


def table(data, tables, tag):
    if tag not in tables:
        raise SystemExit(f"font has no {tag.decode()} table")
    offset, length = tables[tag]
    return data[offset : offset + length]


def parse_cmap(blob):
    """Character to glyph id, from the first format 4 Unicode subtable."""
    count = struct.unpack(">H", blob[2:4])[0]
    chosen = None
    for i in range(count):
        platform, encoding, offset = struct.unpack(
            ">HHI", blob[4 + i * 8 : 12 + i * 8]
        )
        fmt = struct.unpack(">H", blob[offset : offset + 2])[0]
        if fmt == 4 and (platform, encoding) in ((3, 1), (0, 3), (0, 4), (0, 6)):
            chosen = offset
            break
    if chosen is None:
        raise SystemExit("font has no format 4 Unicode cmap subtable")

    seg_x2 = struct.unpack(">H", blob[chosen + 6 : chosen + 8])[0]
    segs = seg_x2 // 2
    base = chosen + 14
    ends = struct.unpack(f">{segs}H", blob[base : base + seg_x2])
    starts_at = base + seg_x2 + 2
    starts = struct.unpack(f">{segs}H", blob[starts_at : starts_at + seg_x2])
    deltas_at = starts_at + seg_x2
    deltas = struct.unpack(f">{segs}h", blob[deltas_at : deltas_at + seg_x2])
    ranges_at = deltas_at + seg_x2
    ranges = struct.unpack(f">{segs}H", blob[ranges_at : ranges_at + seg_x2])

    mapping = {}
    for i in range(segs):
        for code in range(starts[i], min(ends[i], 0xFFFF) + 1):
            if code == 0xFFFF:
                continue
            if ranges[i] == 0:
                gid = (code + deltas[i]) & 0xFFFF
            else:
                at = ranges_at + i * 2 + ranges[i] + (code - starts[i]) * 2
                if at + 2 > len(blob):
                    continue
                gid = struct.unpack(">H", blob[at : at + 2])[0]
                if gid:
                    gid = (gid + deltas[i]) & 0xFFFF
            if gid:
                mapping[code] = gid
    return mapping


def parse_loca(blob, num_glyphs, long_format):
    if long_format:
        return list(struct.unpack(f">{num_glyphs + 1}I", blob[: (num_glyphs + 1) * 4]))
    shorts = struct.unpack(f">{num_glyphs + 1}H", blob[: (num_glyphs + 1) * 2])
    return [s * 2 for s in shorts]


def components_of(glyph):
    """Component glyph ids of a composite glyph, with the offset of each id."""
    if not glyph:
        return []
    contours = struct.unpack(">h", glyph[:2])[0]
    if contours >= 0:
        return []
    out = []
    at = 10
    while True:
        flags, index = struct.unpack(">HH", glyph[at : at + 4])
        out.append((at + 2, index))
        at += 4
        at += 4 if flags & 0x0001 else 2  # ARG_1_AND_2_ARE_WORDS
        if flags & 0x0008:  # WE_HAVE_A_SCALE
            at += 2
        elif flags & 0x0040:  # X_AND_Y_SCALE
            at += 4
        elif flags & 0x0080:  # TWO_BY_TWO
            at += 8
        if not flags & 0x0020:  # MORE_COMPONENTS
            break
    return out


def strip_instructions(glyph):
    """Drop hinting bytecode; we never interpret it."""
    if not glyph:
        return glyph
    contours = struct.unpack(">h", glyph[:2])[0]
    if contours >= 0:
        at = 10 + contours * 2
        length = struct.unpack(">H", glyph[at : at + 2])[0]
        return glyph[:at] + b"\x00\x00" + glyph[at + 2 + length :]
    # Composite: WE_HAVE_INSTRUCTIONS on the last component, then a length and
    # the bytecode, which runs to the end of the glyph.
    at = 10
    while True:
        flags = struct.unpack(">H", glyph[at : at + 2])[0]
        at += 4
        at += 4 if flags & 0x0001 else 2
        if flags & 0x0008:
            at += 2
        elif flags & 0x0040:
            at += 4
        elif flags & 0x0080:
            at += 8
        if not flags & 0x0020:
            break
    if flags & 0x0100:  # WE_HAVE_INSTRUCTIONS
        return glyph[:at]
    return glyph


def build_cmap(mapping):
    """A format 4 subtable, delta-coded, in a single (3, 1) encoding record."""
    codes = sorted(mapping)
    segments = []
    for code in codes:
        gid = mapping[code]
        if segments and code == segments[-1][1] + 1 and gid - code == segments[-1][2]:
            segments[-1][1] = code
        else:
            segments.append([code, code, gid - code])
    segments.append([0xFFFF, 0xFFFF, 1])

    count = len(segments)
    search = 1
    entry = 0
    while search * 2 <= count:
        search *= 2
        entry += 1
    subtable = struct.pack(
        ">HHHHHHH",
        4,
        16 + count * 8,
        0,
        count * 2,
        search * 2,
        entry,
        count * 2 - search * 2,
    )
    subtable += struct.pack(f">{count}H", *(s[1] for s in segments))
    subtable += b"\x00\x00"
    subtable += struct.pack(f">{count}H", *(s[0] for s in segments))
    subtable += struct.pack(f">{count}h", *(((s[2] + 0x8000) % 0x10000) - 0x8000 for s in segments))
    subtable += struct.pack(f">{count}H", *(0 for _ in segments))
    return struct.pack(">HHHHI", 0, 1, 3, 1, 12) + subtable


def checksum(blob):
    padded = blob + b"\x00" * (-len(blob) % 4)
    return sum(struct.unpack(f">{len(padded) // 4}I", padded)) & 0xFFFFFFFF


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__.strip().splitlines()[-4].strip())
    data = open(sys.argv[1], "rb").read()
    tables = read_tables(data)

    head = bytearray(table(data, tables, b"head"))
    hhea = bytearray(table(data, tables, b"hhea"))
    maxp = bytearray(table(data, tables, b"maxp"))
    hmtx = table(data, tables, b"hmtx")
    num_glyphs = struct.unpack(">H", maxp[4:6])[0]
    long_loca = struct.unpack(">h", head[50:52])[0] == 1
    loca = parse_loca(table(data, tables, b"loca"), num_glyphs, long_loca)
    glyf = table(data, tables, b"glyf")
    cmap = parse_cmap(table(data, tables, b"cmap"))

    def glyph_bytes(gid):
        return glyf[loca[gid] : loca[gid + 1]]

    # Which glyphs we need, following composite references to a fixed point.
    keep = {0}
    for code in WANTED:
        if code in cmap:
            keep.add(cmap[code])
    frontier = list(keep)
    while frontier:
        gid = frontier.pop()
        for _, component in components_of(glyph_bytes(gid)):
            if component not in keep:
                keep.add(component)
                frontier.append(component)

    order = sorted(keep)
    remap = {old: new for new, old in enumerate(order)}

    # glyf and loca, with components renumbered and hinting dropped.
    new_glyf = bytearray()
    new_loca = [0]
    for old in order:
        glyph = bytearray(strip_instructions(glyph_bytes(old)))
        for at, component in components_of(bytes(glyph)):
            struct.pack_into(">H", glyph, at, remap[component])
        glyph += b"\x00" * (-len(glyph) % 4)
        new_glyf += glyph
        new_loca.append(len(new_glyf))

    # Horizontal metrics: a full pair for every kept glyph, which is simpler
    # than preserving the "repeat the last advance" tail and costs a few
    # hundred bytes.
    old_metrics = struct.unpack(">H", hhea[34:36])[0]

    def advance_and_lsb(gid):
        if gid < old_metrics:
            return struct.unpack(">Hh", hmtx[gid * 4 : gid * 4 + 4])
        advance = struct.unpack(">H", hmtx[(old_metrics - 1) * 4 : (old_metrics - 1) * 4 + 2])[0]
        at = old_metrics * 4 + (gid - old_metrics) * 2
        lsb = struct.unpack(">h", hmtx[at : at + 2])[0] if at + 2 <= len(hmtx) else 0
        return advance, lsb

    new_hmtx = b"".join(struct.pack(">Hh", *advance_and_lsb(old)) for old in order)

    struct.pack_into(">H", maxp, 4, len(order))
    struct.pack_into(">H", hhea, 34, len(order))
    struct.pack_into(">h", head, 50, 1)  # long loca, so no even-offset constraint
    struct.pack_into(">I", head, 8, 0)  # checkSumAdjustment, filled in below

    new_cmap = build_cmap({c: remap[cmap[c]] for c in WANTED if c in cmap})
    out_tables = {
        b"cmap": bytes(new_cmap),
        b"glyf": bytes(new_glyf),
        b"head": bytes(head),
        b"hhea": bytes(hhea),
        b"hmtx": bytes(new_hmtx),
        b"loca": struct.pack(f">{len(new_loca)}I", *new_loca),
        b"maxp": bytes(maxp),
    }
    assert sorted(out_tables) == KEEP_TABLES

    count = len(out_tables)
    search = 1
    entry = 0
    while search * 2 <= count:
        search *= 2
        entry += 1
    header = struct.pack(
        ">IHHHH", 0x00010000, count, search * 16, entry, count * 16 - search * 16
    )

    offset = 12 + count * 16
    directory = b""
    body = b""
    for tag in KEEP_TABLES:
        blob = out_tables[tag]
        directory += tag + struct.pack(">III", checksum(blob), offset, len(blob))
        padded = blob + b"\x00" * (-len(blob) % 4)
        body += padded
        offset += len(padded)

    font = bytearray(header + directory + body)
    adjustment = (0xB1B0AFBA - checksum(bytes(font))) & 0xFFFFFFFF
    head_offset = 12 + count * 16
    for tag in KEEP_TABLES:
        if tag == b"head":
            break
        head_offset += len(out_tables[tag]) + (-len(out_tables[tag]) % 4)
    struct.pack_into(">I", font, head_offset + 8, adjustment)

    open(sys.argv[2], "wb").write(bytes(font))
    print(
        f"{sys.argv[2]}: {len(order)} glyphs, {len(font)} bytes "
        f"(from {num_glyphs} glyphs, {len(data)} bytes)"
    )


if __name__ == "__main__":
    main()
