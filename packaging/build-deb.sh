#!/usr/bin/env bash
#
# Builds the binary package for Debian, Ubuntu and their derivatives into
# dist/.
#
# Everything the package contains comes out of `make install`. This only wraps
# what that produced, so there is exactly one description of where a file goes
# and it is the Makefile. The library dependencies are read off the binary by
# dpkg-shlibdeps rather than written down by hand.
#
# Needs: make, cargo (Rust 1.88 or newer), clang, pkg-config, the -dev packages
# release.yml installs, dpkg-dev.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
version="${1:-$(make -s -C "$here" version)}"
# The package depends on the GTK and libadwaita of the distribution it is built
# on, so each one gets a build of its own, told apart by a suffix: ~deb13,
# ~ubuntu26.04.
debversion="$version${DEB_SUFFIX:-}"
name=voicenux

for tool in cargo dpkg-deb dpkg-shlibdeps; do
	command -v "$tool" > /dev/null || { echo "$0: $tool is not installed" >&2; exit 1; }
done

root="$(mktemp -d)"
work="$(mktemp -d)"
trap 'rm -rf -- "$root" "$work"' EXIT

make -C "$here" build
make -C "$here" install DESTDIR="$root" PREFIX=/usr

arch="$(dpkg --print-architecture)"

# The shared libraries the binary needs, as package names.
mkdir -p "$work/debian"
printf 'Source: %s\n\nPackage: %s\nArchitecture: any\n' "$name" "$name" > "$work/debian/control"
depends="$(cd "$work" && dpkg-shlibdeps -O -e"$root/usr/bin/$name" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')"
[[ -n $depends ]] || { echo "$0: dpkg-shlibdeps found no dependencies" >&2; exit 1; }

install -d "$root/DEBIAN"
sed -e "s|@VERSION@|$debversion|g" -e "s|@ARCH@|$arch|g" -e "s|@DEPENDS@|$depends|g" \
	"$here/packaging/deb/control" > "$root/DEBIAN/control"
install -Dm644 "$here/packaging/deb/copyright" "$root/usr/share/doc/$name/copyright"

( cd "$root" && find . -type f ! -path './DEBIAN/*' -printf '%P\0' \
	| LC_ALL=C sort -z | xargs -0 md5sum > DEBIAN/md5sums )

mkdir -p "$here/dist"
out="$here/dist/${name}_${debversion}_${arch}.deb"
dpkg-deb --root-owner-group --build "$root" "$out" > /dev/null

echo "$out"
