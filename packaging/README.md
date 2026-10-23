# Packaging and releases

The Makefile installs everything; these only wrap what it produced. That is
on purpose: a packaging script that lists the files again is a second
description of the layout, and two descriptions drift.

| | |
|---|---|
| `deb/control`, `deb/copyright` | metadata for the Debian binary package |
| `rpm/voicenux.spec` | the RPM spec |
| `build-deb.sh`, `build-rpm.sh`, `build-tarball.sh`, `build-flatpak.sh` | build one package into `dist/` |
| `flatpak/` | the Flatpak manifest |
| `check-version.sh` | refuses a tag that disagrees with `Cargo.toml` |
| `publish-repos.sh` | regenerates the APT and RPM repositories |
| `pages/` | the landing page and the `.repo` file served from GitHub Pages |

Voicenux is one binary: the pictures, sounds and styles are built into it.
The packages are per architecture (amd64 and x86_64). Their library
dependencies are read off the binary: by `dpkg-shlibdeps` for the deb, by RPM
on its own. Besides the libraries, they want PipeWire and WirePlumber running.

The `.deb` is built twice, in Debian 13 and in Ubuntu 26.04, with a suffix on
the version (`~deb13`, `~ubuntu26.04`), since each fits the GTK and libadwaita
it was built against. Each gets an APT repository of its own: `deb/trixie`
and `deb/resolute`. The RPM is built on the current Fedora. The Rust of Debian
13 is too old for the code, so the deb builds take Rust from rustup.

The Arch tarball (`voicenux-<version>-arch-x86_64.tar.zst`) is built on Arch.
It holds a folder with the `make install` tree, for an AUR package
`voicenux-bin`.

The Flatpak (`voicenux-x86_64.flatpak`) is one bundle file on the GNOME runtime
from Flathub, without the version in its name, so the README can link the one
of the latest release. Its manifest builds offline, from `cargo-sources.json`
that `build-flatpak.sh` makes from `Cargo.lock` first. That is how Flathub
wants Rust apps, so the same manifest can go there. In the sandbox, autostart
goes through the background portal. The API can not see which app connects
from inside it, so there every app counts as unknown.

## Building one by hand

```bash
packaging/build-deb.sh      # in a Debian 13 container, with the -dev packages from release.yml
packaging/build-rpm.sh      # in a Fedora container, with the -devel packages from the spec
packaging/build-tarball.sh  # in an Arch container, with the packages from release.yml
packaging/build-flatpak.sh  # anywhere with flatpak and flatpak-builder
```

The first three take the version from `make version` unless one is passed as
the first argument.

## Making a release

See "Releases" in CLAUDE.md. In short: bump the version in `Cargo.toml`, add
the entry to `CHANGELOG.md` and the metainfo, commit, then
`git tag vX.Y.Z && git push origin main vX.Y.Z`.

The `release` workflow builds the packages in Debian, Ubuntu, Fedora and Arch
containers and the Flatpak, installs each one and runs `voicenux --version`, refuses the tag
if it disagrees with `Cargo.toml` or has no changelog entry, and attaches the
packages to a GitHub release with the entry as its notes.

## The repositories

With a `GPG_PRIVATE_KEY` secret, the workflow also signs the RPM and adds the
deb and rpm files to the APT and RPM repositories on the `gh-pages` branch
(GitHub Pages, from that branch). Without one it says so and skips them: the
packages are on the release either way.

## Trying the release path first

```bash
gh workflow run release.yml -f dry_run=true
```

This builds everything, signs with a throwaway key, builds the repositories
and installs from them, without publishing anything.
