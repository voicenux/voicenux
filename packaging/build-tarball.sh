#!/usr/bin/env bash
#
# Builds the tarball for Arch Linux into dist/. The AUR package voicenux-bin is
# made from it.
#
# Like the deb and the rpm, it holds what `make install` produced, under a
# folder named like the tarball. It is built on Arch, with the flags Arch
# builds its own packages with, so it fits the GTK and libadwaita there.
#
# Needs: make, cargo, clang, the packages check.yml installs, zstd.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
version="${1:-$(make -s -C "$here" version)}"
name="voicenux-$version-arch-$(uname -m)"

# The flags Arch builds its own packages with: hardening, and full RELRO.
if [[ -f /etc/makepkg.conf ]]; then
	# shellcheck source=/dev/null
	source /etc/makepkg.conf
	export CFLAGS CXXFLAGS LDFLAGS RUSTFLAGS
fi

for tool in cargo zstd; do
	command -v "$tool" > /dev/null || { echo "$0: $tool is not installed" >&2; exit 1; }
done

root="$(mktemp -d)"
trap 'rm -rf -- "$root"' EXIT
dest="$root/$name"

make -C "$here" build
make -C "$here" install DESTDIR="$dest" PREFIX=/usr

mkdir -p "$here/dist"
out="$here/dist/$name.tar.zst"
tar -C "$root" --owner=0 --group=0 --numeric-owner --sort=name \
	-I 'zstd -19 -T0' -cf "$out" "$name"

echo "$out"
