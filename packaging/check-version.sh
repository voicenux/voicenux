#!/usr/bin/env bash
#
# Refuses a release whose tag and Cargo.toml disagree.
#
# The program reports the version of Cargo.toml (`voicenux --version`), and the
# packages take theirs from the same place. But the tag is what people see and
# what the release is named after. A tag that says something else would make a
# package called 0.2.0 that reports 0.1.0, and nothing would complain.
#
# Anything that is not a v-tag (a run started by hand from a branch) is not a
# release and has nothing to check.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ref="${1:-}"

case "$ref" in
	v[0-9]*) ;;
	*)
		echo "not a release tag (${ref:-none}), nothing to check against"
		exit 0
		;;
esac

tag_version="${ref#v}"
cargo_version="$(make -s -C "$here" version)"

if [[ $tag_version != "$cargo_version" ]]; then
	echo "tag $ref says $tag_version, Cargo.toml says $cargo_version" >&2
	echo "Bump the version in Cargo.toml to match the tag, or retag." >&2
	exit 1
fi

echo "$ref matches Cargo.toml"
