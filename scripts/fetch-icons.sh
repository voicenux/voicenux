#!/bin/sh
# Downloads the Fluent Emoji 3D pictures (MIT, Microsoft) that voices and sounds use.
# The pictures are committed, so this only runs when the list below changes.
set -eu
base="https://raw.githubusercontent.com/microsoft/fluentui-emoji/main/assets"
out="$(dirname "$0")/../data/icons"
mkdir -p "$out"

while IFS='|' read -r id path; do
	[ -z "$id" ] && continue
	[ -f "$out/$id.png" ] && continue
	echo "$id"
	curl -fsSL -o "$out/$id.png" "$base/$(printf '%s' "$path" | sed 's/ /%20/g')"
done <<'LIST'
baby|Baby/Default/3D/baby_3d_default.png
kid|Child/Default/3D/child_3d_default.png
woman|Woman/Default/3D/woman_3d_default.png
man-beard|Man beard/Default/3D/man_beard_3d_default.png
old-man|Old man/Default/3D/old_man_3d_default.png
old-woman|Old woman/Default/3D/old_woman_3d_default.png
moai|Moai/3D/moai_3d.png
chipmunk|Chipmunk/3D/chipmunk_3d.png
mouse|Mouse face/3D/mouse_face_3d.png
balloon|Balloon/3D/balloon_3d.png
santa|Santa claus/Default/3D/santa_claus_3d_default.png
pirate-flag|Pirate flag/3D/pirate_flag_3d.png
mage|Man mage/Default/3D/man_mage_3d_default.png
fairy|Woman fairy/Default/3D/woman_fairy_3d_default.png
studio-microphone|Studio microphone/3D/studio_microphone_3d.png
clapper-board|Clapper board/3D/clapper_board_3d.png
horns|Smiling face with horns/3D/smiling_face_with_horns_3d.png
ogre|Ogre/3D/ogre_3d.png
zombie|Man zombie/3D/man_zombie_3d.png
ghost|Ghost/3D/ghost_3d.png
vampire|Man vampire/Default/3D/man_vampire_3d_default.png
witch|Woman mage/Default/3D/woman_mage_3d_default.png
dragon|Dragon/3D/dragon_3d.png
goblin|Goblin/3D/goblin_3d.png
skull|Skull/3D/skull_3d.png
supervillain|Man supervillain/Default/3D/man_supervillain_3d_default.png
clown|Clown face/3D/clown_face_3d.png
robot|Robot/3D/robot_3d.png
alien|Alien/3D/alien_3d.png
mechanical-arm|Mechanical arm/3D/mechanical_arm_3d.png
alien-monster|Alien monster/3D/alien_monster_3d.png
astronaut|Astronaut/Default/3D/astronaut_3d_default.png
television|Television/3D/television_3d.png
crystal-ball|Crystal ball/3D/crystal_ball_3d.png
mountain|Snow-capped mountain/3D/snow-capped_mountain_3d.png
rock|Rock/3D/rock_3d.png
church|Church/3D/church_3d.png
shower|Shower/3D/shower_3d.png
stadium|Stadium/3D/stadium_3d.png
diving-mask|Diving mask/3D/diving_mask_3d.png
ringed-planet|Ringed planet/3D/ringed_planet_3d.png
storm|Cloud with lightning and rain/3D/cloud_with_lightning_and_rain_3d.png
fire|Fire/3D/fire_3d.png
radio|Radio/3D/radio_3d.png
telephone|Telephone receiver/3D/telephone_receiver_3d.png
antenna|Satellite antenna/3D/satellite_antenna_3d.png
megaphone|Megaphone/3D/megaphone_3d.png
top-hat|Top hat/3D/top_hat_3d.png
potato|Potato/3D/potato_3d.png
video-game|Video game/3D/video_game_3d.png
cyclone|Cyclone/3D/cyclone_3d.png
pilot|Man pilot/Default/3D/man_pilot_3d_default.png
microphone|Microphone/3D/microphone_3d.png
angel|Baby angel/Default/3D/baby_angel_3d_default.png
keyboard|Musical keyboard/3D/musical_keyboard_3d.png
control-knobs|Control knobs/3D/control_knobs_3d.png
bunny-ears|Person with bunny ears/3D/person_with_bunny_ears_3d.png
woozy|Woozy face/3D/woozy_face_3d.png
spiral-eyes|Face with spiral eyes/3D/face_with_spiral_eyes_3d.png
shushing|Shushing face/3D/shushing_face_3d.png
cat|Cat face/3D/cat_face_3d.png
troll|Troll/3D/troll_3d.png
frog|Frog/3D/frog_3d.png
genie|Man genie/3D/man_genie_3d.png
superhero|Person superhero/Default/3D/person_superhero_3d_default.png
detective|Man detective/Default/3D/man_detective_3d_default.png
unicorn|Unicorn/3D/unicorn_3d.png
owl|Owl/3D/owl_3d.png
wolf|Wolf/3D/wolf_3d.png
dog|Dog face/3D/dog_face_3d.png
pig|Pig face/3D/pig_face_3d.png
cow|Cow face/3D/cow_face_3d.png
duck|Duck/3D/duck_3d.png
goat|Goat/3D/goat_3d.png
rooster|Rooster/3D/rooster_3d.png
teddy-bear|Teddy bear/3D/teddy_bear_3d.png
pumpkin|Jack-o-lantern/3D/jack-o-lantern_3d.png
spider|Spider/3D/spider_3d.png
bat|Bat/3D/bat_3d.png
snowman|Snowman/3D/snowman_3d.png
microbe|Microbe/3D/microbe_3d.png
brain|Brain/3D/brain_3d.png
sparkles|Sparkles/3D/sparkles_3d.png
rocket|Rocket/3D/rocket_3d.png
headphone|Headphone/3D/headphone_3d.png
musical-notes|Musical notes/3D/musical_notes_3d.png
joy|Face with tears of joy/3D/face_with_tears_of_joy_3d.png
crying|Crying face/3D/crying_face_3d.png
thinking|Thinking face/3D/thinking_face_3d.png
postal-horn|Postal horn/3D/postal_horn_3d.png
trumpet|Trumpet/3D/trumpet_3d.png
drum|Drum/3D/drum_3d.png
clapping|Clapping hands/Default/3D/clapping_hands_3d_default.png
collision|Collision/3D/collision_3d.png
bomb|Bomb/3D/bomb_3d.png
party-popper|Party popper/3D/party_popper_3d.png
cross-mark|Cross mark/3D/cross_mark_3d.png
bell|Bell/3D/bell_3d.png
cricket|Cricket/3D/cricket_3d.png
dashing-away|Dashing away/3D/dashing_away_3d.png
coin|Coin/3D/coin_3d.png
high-voltage|High voltage/3D/high_voltage_3d.png
loudspeaker|Loudspeaker/3D/loudspeaker_3d.png
level|Level slider/3D/level_slider_3d.png
basketball|Basketball/3D/basketball_3d.png
gear|Gear/3D/gear_3d.png
alembic|Alembic/3D/alembic_3d.png
keyboard-2|Keyboard/3D/keyboard_3d.png
magnifying-glass-tilted-left|Magnifying glass tilted left/3D/magnifying_glass_tilted_left_3d.png
globe-with-meridians|Globe with meridians/3D/globe_with_meridians_3d.png
lady-beetle|Lady beetle/3D/lady_beetle_3d.png
hot-beverage|Hot beverage/3D/hot_beverage_3d.png
scissors|Scissors/3D/scissors_3d.png
speaker-high-volume|Speaker high volume/3D/speaker_high_volume_3d.png
unamused-face|Unamused face/3D/unamused_face_3d.png
broken-heart|Broken heart/3D/broken_heart_3d.png
warning|Warning/3D/warning_3d.png
face-with-hand-over-mouth|Face with hand over mouth/3D/face_with_hand_over_mouth_3d.png
violin|Violin/3D/violin_3d.png
nut-and-bolt|Nut and bolt/3D/nut_and_bolt_3d.png
smirking-face|Smirking face/3D/smirking_face_3d.png
star-struck|Star-struck/3D/star-struck_3d.png
face-screaming-in-fear|Face screaming in fear/3D/face_screaming_in_fear_3d.png
speech-balloon|Speech balloon/3D/speech_balloon_3d.png
pile-of-poo|Pile of poo/3D/pile_of_poo_3d.png
sponge|Sponge/3D/sponge_3d.png
star|Star/3D/star_3d.png
octopus|Octopus/3D/octopus_3d.png
crossed-swords|Crossed swords/3D/crossed_swords_3d.png
lizard|Lizard/3D/lizard_3d.png
articulated-lorry|Articulated lorry/3D/articulated_lorry_3d.png
test-tube|Test tube/3D/test_tube_3d.png
cucumber|Cucumber/3D/cucumber_3d.png
doughnut|Doughnut/3D/doughnut_3d.png
LIST
