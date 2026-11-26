#!/usr/bin/env bash
# Build the site twice, from two different absolute paths, and compare.
#
# ADR-0004 Decision 4 item 1, applied to the half of the build that exists
# today. The rule it enforces: we do not publish a hash for a build that is not
# in fact reproducible. A release that cannot reproduce itself does not ship.
#
# The second build runs from a different directory on purpose. For the Rust
# side that is the variable ADR-0004's finding 5 shows can silently reintroduce
# path dependence; for the JavaScript side it is the cheapest way to catch a
# tool that embeds its own working directory into a source map, a chunk name or
# a comment. MEASURED on this tree before the check existed: identical, with
# zero occurrences of the builder's home directory anywhere in `dist/`.
#
# The comparison is on `files_digest` from the manifest, which covers every
# built byte and nothing about the machine. The manifest's own `build` section
# records node and pnpm versions and is deliberately excluded from that digest.
#
# Usage: scripts/check-web-reproducible.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/pochoir-web-reproduce.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

# A deliberately differently-shaped path, so a leaked prefix cannot match by
# coincidence of length.
second="$scratch/a-second-checkout-at-another-depth/web"
mkdir -p "$(dirname "$second")"

echo "==> build 1: $repo_root/web"
rm -rf "$repo_root/web/dist" "$repo_root/web/.astro"
pnpm --dir "$repo_root/web" run build >/dev/null
"$repo_root/scripts/web-attest.py" "$repo_root/web/dist" --quiet >/dev/null
digest_one="$(python3 -c '
import json, sys
print(json.load(open(sys.argv[1]))["files_digest"])
' "$repo_root/web/dist/manifest.json")"
echo "    files_digest $digest_one"

echo "==> build 2: $second"
# Copy the sources only. node_modules is a symlink farm that does not survive a
# copy intact, and dist/.astro are outputs.
mkdir -p "$second"
(cd "$repo_root/web" && tar --exclude=node_modules --exclude=dist --exclude=.astro -cf - .) \
  | (cd "$second" && tar -xf -)
pnpm --dir "$second" install --frozen-lockfile >/dev/null 2>&1
pnpm --dir "$second" run build >/dev/null
"$repo_root/scripts/web-attest.py" "$second/dist" --quiet >/dev/null
digest_two="$(python3 -c '
import json, sys
print(json.load(open(sys.argv[1]))["files_digest"])
' "$second/dist/manifest.json")"
echo "    files_digest $digest_two"

echo "==> leaked absolute paths"
leaks="$(grep -rlF "$repo_root" "$repo_root/web/dist" 2>/dev/null || true)"
if [ -n "$leaks" ]; then
  echo "    FAILED: the build output names its own checkout directory:" >&2
  echo "$leaks" >&2
  exit 1
fi
echo "    none"

if [ "$digest_one" != "$digest_two" ]; then
  echo >&2
  echo "check-web-reproducible: FAILED — two builds of the same source disagree." >&2
  echo "  build 1 $digest_one" >&2
  echo "  build 2 $digest_two" >&2
  echo >&2
  echo "Per-file differences:" >&2
  diff <(python3 -c '
import json, sys
m = json.load(open(sys.argv[1]))["files"]
for k in sorted(m):
    print(k, m[k]["sha384"])
' "$repo_root/web/dist/manifest.json") \
       <(python3 -c '
import json, sys
m = json.load(open(sys.argv[1]))["files"]
for k in sorted(m):
    print(k, m[k]["sha384"])
' "$second/dist/manifest.json") >&2 || true
  exit 1
fi

echo
echo "check-web-reproducible: two builds at different paths agree."
echo "  files_digest $digest_one"
