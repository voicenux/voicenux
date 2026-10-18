#!/usr/bin/env bash
#
# Builds the binary package for Fedora into dist/.
#
# The spec takes the version as a macro rather than carrying one of its own,
# for the same reason the Debian control file has a placeholder: Cargo.toml is
# where the version is written down.
#
# Needs: rpmbuild, make, cargo, clang, the -devel packages the spec lists.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
version="${1:-$(make -s -C "$here" version)}"
name=voicenux

command -v rpmbuild > /dev/null || { echo "$0: rpmbuild is not installed" >&2; exit 1; }

top="$(mktemp -d)"
trap 'rm -rf -- "$top"' EXIT
mkdir -p "$top"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS}

# The working tree as it is, not as it was committed: a package built from a
# checkout has to contain what is in that checkout.
tar czf "$top/SOURCES/$name-$version.tar.gz" \
	--transform "s,^\\.,$name-$version," \
	--exclude=./.git --exclude=./dist --exclude=./target \
	-C "$here" .

rpmbuild \
	--define "_topdir $top" \
	--define "_version $version" \
	-bb "$here/packaging/rpm/$name.spec" > /dev/null

mkdir -p "$here/dist"
find "$top/RPMS" -name '*.rpm' -exec cp {} "$here/dist/" \;

ls "$here/dist/$name-$version"*.rpm
