#!/usr/bin/env bash
# Builds rfiles_<version>_<arch>.deb and rfiles-<version>.<arch>.rpm into dist/ from
# binaries already built with `cargo build --release -p rf-app --bins`.
#
#   packaging/linux/build-packages.sh [target/release]
#
# Needs nfpm (https://nfpm.goreleaser.com) on PATH. Run from anywhere in the repo.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
bin_dir=${1:-${CARGO_TARGET_DIR:-target}/release}
for b in rfiles directlook; do
  [[ -x $bin_dir/$b ]] || { echo "missing $bin_dir/$b: build with cargo build --release -p rf-app --bins" >&2; exit 1; }
done

version=$(packaging/linux/version.sh)

case $(uname -m) in
  x86_64) arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

# the newest glibc symbol the binaries use: the oldest glibc they run on
glibc=$(objdump -T "$bin_dir/rfiles" "$bin_dir/directlook" | grep -oE 'GLIBC_[0-9.]+' | sed 's/GLIBC_//' | sort -Vu | tail -n1)

# nfpm expands environment variables in only some fields (not file paths): fill them in here
mkdir -p dist
config=$(mktemp --suffix=.yaml)
trap 'rm -f "$config"' EXIT
sed -e "s|\${RF_VERSION}|$version|g" -e "s|\${RF_ARCH}|$arch|g" \
  -e "s|\${RF_BIN_DIR}|$bin_dir|g" -e "s|\${RF_GLIBC}|$glibc|g" packaging/linux/nfpm.yaml >"$config"
if grep -n '\${RF_' "$config"; then echo "unfilled variable in nfpm.yaml" >&2; exit 1; fi
for fmt in deb rpm; do
  nfpm package --config "$config" --packager "$fmt" --target dist/
done
ls -l dist/
