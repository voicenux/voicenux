#!/bin/sh
# Makes the background loops in data/ambience from Freesound recordings, all CC0 (see
# data/ambience/SOURCES.md): a part of each, mono, leveled. The loops are committed, so this
# only runs when the list below changes. Needs curl and ffmpeg with libvorbis.
set -eu
export LC_ALL=C
out="$(dirname "$0")/../data/ambience"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Name, preview, start and length in seconds, loudness in LUFS or a gain in dB for sounds with
# single loud pops (a limiter would flatten them), and an extra filter.
while read -r name preview start len level extra; do
	[ -z "$name" ] && continue
	echo "$name"
	curl -sfL -o "$tmp/in.ogg" "https://cdn.freesound.org/previews/$preview-hq.ogg"
	f="atrim=$start:$((start + len)),asetpts=N/SR/TB,aformat=channel_layouts=mono,highpass=f=25"
	[ "$extra" != "-" ] && f="$f,$extra"
	f="$f,aresample=48000"
	case "$level" in
	*dB) f="$f,volume=$level" ;;
	*)
		i=$(ffmpeg -nostdin -nostats -i "$tmp/in.ogg" -af "$f,ebur128" -f null - 2>&1 | awk '/Summary/ {s = 1} s && /I:/ {print $2; exit}')
		f="$f,volume=$(awk "BEGIN {print $level - ($i)}")dB,alimiter=limit=0.7:attack=2:release=80:level=false"
		;;
	esac
	ffmpeg -nostdin -v error -y -i "$tmp/in.ogg" -af "$f" -c:a libvorbis -q:a 2 "$out/$name.ogg"
done <<EOF
bubbles 852/852478_18763192 0 45 -18 -
wind 84/84914_854782 0 45 -18 -
rain 243/243781_997601 0 40 -18 -
campfire 637/637523_612689 0 35 1.8dB -
ocean 827/827530_2718606 10 50 -18 -
space 234/234316_3840537 0 42 -18 -
static 110/110739_1058945 2 45 -18 -
drips 177/177958_985466 0 60 6.4dB -
heartbeat 332/332819_5859881 0 30 -18 -
computer 729/729390_14953219 0 45 -18 -
storm 278/278867_1402315 0 121 -18 acompressor=threshold=-24dB:ratio=3:attack=5:release=400
crowd 424/424354_2639035 0 62 -18 -
birds 474/474342_9461190 0 60 -18 highpass=f=200
engine 451/451668_612689 0 36 -18 -
shower 442/442845_2237208 2 45 -18 -
cauldron 506/506505_6142149 0 39 -18 -
EOF
