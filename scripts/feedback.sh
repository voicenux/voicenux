#!/bin/sh
# Makes the sounds of the switches in data/feedback from Kenney's Interface Sounds 1.0, CC0 (see
# data/feedback/SOURCES.md): mono, 48 kHz. They are committed, so this only runs when they change.
# Get the pack from https://kenney.nl/assets/interface-sounds (or OpenGameArt) and pass its Audio
# folder. Needs ffmpeg with libvorbis.
#
# Both are one soft "tock", played slower so it is deeper: a little higher for on, lower for off.
# The highs go, and it ends after 90 ms, so it does not get in the way.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 <Audio folder of Interface Sounds>" >&2; exit 1; }
out="$(dirname "$0")/../data/feedback"
audio="$1"
for pair in "on 28800" "off 21600"; do
	set -- $pair
	ffmpeg -nostdin -v error -y -i "$audio/drop_003.ogg" \
		-af "aformat=channel_layouts=mono,asetrate=$2,aresample=48000,lowpass=f=1800,afade=t=in:d=0.002,afade=t=out:st=0.05:d=0.04,atrim=0:0.09,volume=-7.9dB" \
		-c:a libvorbis -q:a 4 "$out/$1.ogg"
done
