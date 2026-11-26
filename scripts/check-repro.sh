#!/bin/sh
#
# Reproducibility check for the wasm artefact. Run by `make wasm-repro`.
#
# ADR-0004 Decision 1 claims that a pinned toolchain plus two `RUSTFLAGS`
# entries is enough to make the browser module byte-reproducible, with no
# container and no extra tooling. That claim was made when there was no non-empty
# module to test it on: `make wasm` built `pochoir-core`, which exports nothing,
# and produced 43 bytes. This script is the measurement, now that there is
# something to measure.
#
# It builds the same commit twice from two *different absolute paths*, which is
# the variable the decision rests on, and compares the SHA-256 of the two
# artefacts. It also greps the module for the builder's own paths, because the
# second half of the decision is that nothing in the output names the machine it
# was built on.
#
# Decision 1's finding 5 is the trap this guards: a wasm crate whose
# `pochoir-core` path dependency reaches outside the workspace root puts an
# absolute path into `-Cmetadata`, renames every symbol, and yields two
# artefacts of identical size with thousands of bytes different. Nothing in the
# output warns you, which is why this is a script and not a paragraph.

set -eu

die() {
	echo "check-repro: $*" >&2
	exit 1
}

[ -f Cargo.toml ] || die "run me from the repository root."
command -v shasum >/dev/null 2>&1 || command -v sha256sum >/dev/null 2>&1 ||
	die "need shasum or sha256sum."

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	else
		shasum -a 256 "$1" | cut -d' ' -f1
	fi
}

ARTEFACT="target/wasm32-unknown-unknown/release/pochoir_wasm.wasm"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM

# Two copies at deliberately different path lengths, since a longer path is what
# would change the size of an embedded string if one leaked through.
ONE="$WORK/a"
TWO="$WORK/bbbbbbbbbbbbbbbbbbbbbbbb/cc"
mkdir -p "$ONE" "$TWO"

echo "check-repro: copying the working tree to two paths under $WORK"
# Tracked files *and* untracked ones that `.gitignore` does not exclude, which
# together are what the developer is actually building. `git archive` of HEAD
# would miss uncommitted work, and a plain `cp -R` would drag `target/` along
# and defeat the point of building from clean.
git ls-files -z --cached --others --exclude-standard | while IFS= read -r -d '' file; do
	[ -f "$file" ] || continue
	for dest in "$ONE" "$TWO"; do
		mkdir -p "$dest/$(dirname "$file")"
		cp "$file" "$dest/$file"
	done
done

build_in() {
	dir="$1"
	( cd "$dir" && make wasm >"$WORK/log.$(basename "$dir")" 2>&1 ) ||
		{ cat "$WORK/log.$(basename "$dir")" >&2; die "build failed in $dir"; }
	sha256 "$dir/$ARTEFACT"
}

echo "check-repro: building in $ONE"
HASH_ONE=$(build_in "$ONE")
echo "check-repro: building in $TWO"
HASH_TWO=$(build_in "$TWO")

SIZE_ONE=$(wc -c <"$ONE/$ARTEFACT" | tr -d ' ')
SIZE_TWO=$(wc -c <"$TWO/$ARTEFACT" | tr -d ' ')

echo ""
echo "  path A: $ONE"
echo "    $SIZE_ONE bytes  $HASH_ONE"
echo "  path B: $TWO"
echo "    $SIZE_TWO bytes  $HASH_TWO"
echo ""

if [ "$HASH_ONE" != "$HASH_TWO" ]; then
	echo "check-repro: FAIL - the two builds differ." >&2
	echo "" >&2
	echo "  ADR-0004 Decision 1 says a pinned toolchain plus the two RUSTFLAGS" >&2
	echo "  entries in the Makefile's wasm target is sufficient. If this fails," >&2
	echo "  that decision is wrong and its runner-up - a digest-pinned container" >&2
	echo "  - becomes the decision, in a superseding ADR. Do not work around it:" >&2
	echo "  check first that crates/pochoir-wasm is still a member of the root" >&2
	echo "  workspace (Decision 1, finding 5)." >&2
	exit 1
fi

# The second half of Decision 1: no host paths in the shipped bytes. These are
# `core::panic::Location` strings in the data section, which `strip` leaves.
leaked=0
for needle in "$HOME" "$ONE" "$TWO" "/Users/" "/home/"; do
	if LC_ALL=C grep -q -- "$needle" "$ONE/$ARTEFACT" 2>/dev/null; then
		echo "check-repro: FAIL - the artefact contains '$needle'." >&2
		leaked=1
	fi
done
[ "$leaked" -eq 0 ] || {
	echo "" >&2
	echo "  The --remap-path-prefix entries in the Makefile's wasm target are" >&2
	echo "  not covering every path that reaches the data section." >&2
	exit 1
}

echo "check-repro: OK - byte-identical from two paths, and no host path in the module."
