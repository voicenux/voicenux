#!/bin/sh
# Makes the sounds of the switches in data/feedback from Kenney's Interface Sounds 1.0, CC0 (see
# data/feedback/SOURCES.md): mono, 48 kHz. They are committed, so this only runs when they change.
# Get the pack from https://kenney.nl/assets/interface-sounds and pass its Audio folder.
# Needs ffmpeg with libvorbis.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 <Audio folder of Interface Sounds>" >&2; exit 1; }
out="$(dirname "$0")/../data/feedback"
audio="$1"
for pair in "on maximize_006" "off minimize_006"; do
	set -- $pair
	ffmpeg -nostdin -v error -y -i "$audio/$2.ogg" -af aformat=channel_layouts=mono,aresample=48000 -c:a libvorbis -q:a 4 "$out/$1.ogg"
done
