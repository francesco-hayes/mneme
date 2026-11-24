#!/usr/bin/env bash
# Sets the app version everywhere it is written: Cargo.toml
# ([workspace.package]), package.json, and the workspace crates in Cargo.lock.
#
# Usage:
#   pnpm bump 0.2.0
#   pnpm bump 0.2.0-beta.1     a beta: its draft is marked as a pre-release
#
# Commit the result and push it to main. The Release workflow sees the new
# version, builds the app and makes a draft release for v0.2.0 (see
# docs/release.md, "GitHub release").
#
# Exit codes: 0 done, 1 something failed, 2 bad usage.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ $# -ne 1 ] || [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
    [ $# -eq 1 ] && exit 0
    exit 2
fi

new="${1#v}"
if ! [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "Not a version: $1 (expected something like 0.2.0)" >&2
    exit 2
fi

old="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')"
if [ "$new" = "$old" ]; then
    echo "The version is already $new." >&2
    exit 1
fi

# Cargo.toml: only the version line inside [workspace.package].
python3 - "$new" <<'EOF'
import re, sys
new = sys.argv[1]
text = open("Cargo.toml").read()
pattern = re.compile(r'(\[workspace\.package\][^\[]*?^version\s*=\s*")[^"]*(")', re.M | re.S)
text, count = pattern.subn(lambda m: m.group(1) + new + m.group(2), text)
if count != 1:
    sys.exit("Could not find the version under [workspace.package] in Cargo.toml")
open("Cargo.toml", "w").write(text)
EOF

npm pkg set version="$new" > /dev/null
cargo update --workspace --offline --quiet

echo "Version $old -> $new in Cargo.toml, package.json and Cargo.lock."
echo "Next: commit and push to main. The Release workflow makes a draft release for v$new."
