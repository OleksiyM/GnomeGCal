#!/usr/bin/env bash
# Verifies that Cargo.toml version matches the git tag (if given as $1, e.g. v1.0.0).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo_version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)

[[ "$cargo_version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
    echo "Cargo.toml version is not X.Y.Z: $cargo_version" >&2; exit 1; }

if [[ -n "${1:-}" ]]; then
    [[ "${1#v}" == "$cargo_version" ]] || {
        echo "Tag $1 does not match version $cargo_version" >&2; exit 1; }
fi

echo "$cargo_version"
