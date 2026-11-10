#!/usr/bin/env bash
# Prints the package version: a release's tag (HEAD tagged v0.6.0 → 0.6.0), else the
# workspace version plus the commit count and commit (0.5.2+git300.gabc1234), the same
# scheme as the Arch package. Run inside the repository.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
if tag=$(git describe --exact-match --tags --match 'v[0-9]*' HEAD 2>/dev/null); then
  echo "${tag#v}"
  exit 0
fi
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
echo "$ver+git$(git rev-list --count HEAD).g$(git rev-parse --short=7 HEAD)"
