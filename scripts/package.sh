#!/usr/bin/env bash
# Builds the release tarball for this machine's architecture into dist/, with SHA256SUMS-<arch>.
# CI runs the same script.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
arch="$(uname -m)"
dist="$root/dist"
stage="$root/target/package"

rm -rf -- "$dist" "$stage"
mkdir -p "$dist" "$stage"
cargo build --release --locked -p kirie

bundle="kirie-$version-$arch-linux"
mkdir -p "$stage/$bundle"
cp target/release/kirie README.md LICENSE "$stage/$bundle/"
cp -r packaging "$stage/$bundle/packaging"
rm -f -- "$stage/$bundle/packaging/minisign.pub"
tar -C "$stage" -czf "$dist/$bundle.tar.gz" "$bundle"

(cd "$dist" && sha256sum -- * >"SHA256SUMS-$arch")
ls -l "$dist"
