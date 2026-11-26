#!/bin/sh
#
# Dependency budget guard. Run by `make deps-check`, and by CI via `make check`.
#
# This project is deliberately lean on dependencies: we would rather write a
# feature ourselves than inherit a crate we do not control, for reasons of
# supply-chain surface and client bundle size. This script makes that policy
# mechanical instead of aspirational.
#
# THRESHOLD is the number of [[package]] entries allowed in Cargo.lock. It is
# set to exactly the number of crates in this workspace, so pulling in ANY
# third-party crate breaks the build until someone raises it on purpose.
#
# Raising it is a deliberate act: write the ADR under docs/adr/ first, and bump
# the number in the same commit that adds the dependency.

set -eu

THRESHOLD=2

LOCKFILE="${1:-Cargo.lock}"

if [ ! -f "$LOCKFILE" ]; then
	echo "check-deps: '$LOCKFILE' not found. Run 'make build' to generate it." >&2
	exit 1
fi

count=$(grep -c '^\[\[package\]\]' "$LOCKFILE" || true)
: "${count:=0}"

if [ "$count" -gt "$THRESHOLD" ]; then
	echo "check-deps: FAIL" >&2
	echo "  $LOCKFILE contains $count packages, but the budget is $THRESHOLD." >&2
	echo "" >&2
	echo "  Packages currently locked:" >&2
	sed -n 's/^name = "\(.*\)"$/    - \1/p' "$LOCKFILE" >&2
	echo "" >&2
	echo "  A new dependency needs an ADR. Write one under docs/adr/ explaining" >&2
	echo "  why it cannot reasonably be built in-house, then raise THRESHOLD at" >&2
	echo "  the top of scripts/check-deps.sh in the same commit." >&2
	exit 1
fi

echo "check-deps: OK - $count package(s) in $LOCKFILE, budget $THRESHOLD."
