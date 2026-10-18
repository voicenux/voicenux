# Built with `packaging/build-rpm.sh`, which passes the version in rather than
# editing this file: Cargo.toml is where the version is written down.
%global upstream_version %{?_version}%{!?_version:0.1.0}
# Cargo strips the binary already: there is nothing for a debuginfo package.
%global debug_package %{nil}

Name:           voicenux
Version:        %{upstream_version}
Release:        1%{?dist}
Summary:        Voice changer for Linux

# The program is BSD; the emoji in it are MIT, the icons ISC and the
# background sounds CC0.
License:        BSD-3-Clause AND MIT AND ISC AND CC0-1.0
URL:            https://github.com/voicenux/voicenux
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.88
BuildRequires:  rust >= 1.88
BuildRequires:  clang-devel
BuildRequires:  make
BuildRequires:  pkgconfig(gtk4) >= 4.16
BuildRequires:  pkgconfig(libadwaita-1) >= 1.7
BuildRequires:  pkgconfig(libpipewire-0.3)
BuildRequires:  /usr/bin/glib-compile-resources

Requires:       pipewire
Requires:       wireplumber
Recommends:     xdg-desktop-portal

%description
Change your voice in real time in Discord, games, OBS and any other app.
Voicenux adds the Voicenux Microphone to PipeWire: apps that pick it hear your
changed voice.

It comes with 69 voices, from Baby and Robot to Demon and Underwater, AI voices
for an NVIDIA graphics card, background sounds, a soundboard that finds sounds
on MyInstants, and Voicelab to build your own voices from 26 effects. Global
shortcuts, a tray icon and a command line control it.

%prep
%autosetup -n %{name}-%{version}

%build
make

%install
make install DESTDIR=%{buildroot} PREFIX=%{_prefix}

%files
%{_bindir}/%{name}
%{_datadir}/applications/io.github.voicenux.Voicenux.desktop
%{_datadir}/metainfo/io.github.voicenux.Voicenux.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/io.github.voicenux.Voicenux.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.voicenux.Voicenux-symbolic.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.voicenux.Voicenux-background-symbolic.svg
%{_datadir}/licenses/%{name}/

%changelog
