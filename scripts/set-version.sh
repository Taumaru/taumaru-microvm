#!/bin/sh
# Set the workspace version in Cargo.toml and refresh Cargo.lock.
# Used by semantic-release (see .releaserc.json) during the prepare step.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <semver>" >&2
    exit 2
fi

version="$1"
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'; then
    echo "error: '$version' is not a valid semantic version" >&2
    exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$root/Cargo.toml"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

# Replace only the first `version = ...` line inside [workspace.package].
awk -v version="$version" '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") }
    in_pkg && !done && /^version[[:space:]]*=/ {
        print "version = \"" version "\""
        done = 1
        next
    }
    { print }
    END { if (!done) exit 1 }
' "$manifest" >"$tmp" || {
    echo "error: no version field found in [workspace.package] of $manifest" >&2
    exit 1
}
cat "$tmp" >"$manifest"

# Sync the workspace member versions recorded in Cargo.lock.
cargo update --manifest-path "$manifest" --workspace --offline >/dev/null 2>&1 \
    || cargo update --manifest-path "$manifest" --workspace

echo "Workspace version set to $version"
