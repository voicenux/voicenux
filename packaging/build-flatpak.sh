#!/usr/bin/env bash
#
# Builds the Flatpak into dist/, as one bundle file: voicenux-x86_64.flatpak.
# It has no version in its name, so the README can link the one of the latest
# release. People install it with `flatpak install --user`, and the runtime
# comes from Flathub. Flathub can build the same manifest.
#
# The build runs offline, so the crates come in first: cargo-sources.json,
# made from Cargo.lock by flatpak-cargo-generator of flatpak-builder-tools,
# pinned here by commit and hash.
#
# Needs: flatpak, flatpak-builder (or the org.flatpak.Builder Flatpak), curl,
# and python3 with aiohttp and tomlkit (or uv, which brings them).

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
app=io.github.voicenux.Voicenux
manifest="$here/packaging/flatpak/$app.yml"
generator_commit=41c20aa10819cdb2a4f3ca171758a96d1955c018
generator_sha256=0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

curl -fsSL -o "$work/flatpak-cargo-generator.py" \
	"https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$generator_commit/cargo/flatpak-cargo-generator.py"
echo "$generator_sha256  $work/flatpak-cargo-generator.py" | sha256sum -c --quiet
if command -v uv > /dev/null; then
	python=(uv run --no-project --with aiohttp --with tomlkit --with pyyaml python -I)
else
	python=(python3 -I)
fi
"${python[@]}" "$work/flatpak-cargo-generator.py" "$here/Cargo.lock" -o "$here/packaging/flatpak/cargo-sources.json"

if command -v flatpak-builder > /dev/null; then
	builder=(flatpak-builder)
else
	builder=(flatpak run org.flatpak.Builder)
fi
"${builder[@]}" --user --install-deps-from=flathub --force-clean --disable-rofiles-fuse \
	--state-dir="$here/.flatpak-builder" --repo="$here/.flatpak-builder/repo" \
	"$here/.flatpak-builder/build" "$manifest"

mkdir -p "$here/dist"
out="$here/dist/voicenux-$(uname -m).flatpak"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
	"$here/.flatpak-builder/repo" "$out" "$app"

echo "$out"
