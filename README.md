<p align="center">
  <img width="160" src="data/app/io.github.voicenux.Voicenux.svg" alt="Voicenux">
</p>

<h1 align="center">Voicenux</h1>

<h3 align="center">The voice changer for Linux.</h3>

<p align="center">
  Change your voice in real time in Discord, games, OBS and any other app. 68 voices, a soundboard and a lab to build your own. Made for PipeWire.
</p>

<h5 align="center">
  <a href="#install">Install</a> |
  <a href="#how-to-use">How to use</a> |
  <a href="#voices">Voices</a> |
  <a href="https://github.com/voicenux/voicenux/issues">Report a bug</a>
</h5>

<p align="center">
  <a href="https://ko-fi.com/felitendo"><img src="https://storage.ko-fi.com/cdn/kofi5.png?v=6" alt="Buy me a coffee on Ko-fi" height="48"></a>
</p>

<p align="center">
  <img width="720" src="res/voices.png" alt="Voicenux with the Baby voice selected: a grid of voice cards, the voice with its pitch settings on the right, and the switches for voice changer, hear myself and background at the bottom">
</p>

## Install

You need PipeWire, GTK 4.16 and libadwaita 1.7 or newer.

<details>
<summary>From source</summary>

You also need Rust, Clang and the development files of PipeWire, GTK 4 and libadwaita.

```bash
git clone https://github.com/voicenux/voicenux.git
cd voicenux
make
sudo make install
```

`sudo make uninstall` removes it again.

</details>

## How to use

1. Open Voicenux. The welcome shows you your microphone and asks if Voicenux should be the default microphone.
2. In Discord, OBS, your game or your browser, pick **Voicenux Microphone** as the microphone. With **Use as default microphone** on, apps that use the default get it by themselves.
3. Click a voice. Turn on **Hear myself** to hear it. Wear headphones, or your speakers feed back into the microphone.

The switches at the bottom turn the voice changer, hear myself and the background sounds of a voice on or off. The button on the left mutes your microphone. On the right you see which apps use your new voice right now.

Every voice has a few settings on the right, like pitch or how big the room is. **Customize** makes a copy in Voicelab, where you can change everything.

When you close the window, Voicenux keeps running in the tray, so your voice keeps working.

### Soundboard

<img width="720" src="res/soundboard.png" alt="The soundboard with sound pads; Drum Roll and Applause are playing and show their progress as a ring. On the right, Find sounds lists applause sounds from MyInstants, each with a play and an add button">

Sounds play into the Voicenux Microphone, and you hear them too. Add your own with **Add sounds**, or drop audio files on the page. MP3, OGG, FLAC and WAV work. The menu of a sound changes its name, picture, volume and shortcut, and what a press does: play from the start, play and stop, play on top or loop. Right-click a sound to open it.

<img width="720" src="res/trim.png" alt="Trim sound for Drum Roll: the waveform with a blue frame and a handle on each side, from 0.80 s to 5.20 s, and a playhead while it plays">

**Trim sound** in the menu cuts the start and end, like a video clip. Drag the handles over the waveform, or type the times. **Listen** plays the cut sound in your headphones only, from the white playhead: drag it, or click into the cut. Space listens or pauses, ← and → move the playhead, I and O set the start and end at the playhead.

**Find sounds** searches [MyInstants](https://www.myinstants.com). ▶ plays a sound in your headphones only, + adds it to your soundboard. Only the words you search for are sent, and only when you search.

### Voicelab

<img width="720" src="res/voicelab.png" alt="Voicelab with a custom voice called Space Pirate: a pitch effect and a ring modulator, each with its settings">

Build your own voice from 25 effects: pitch and formant, harmony, robot, whisper, auto-tune, vocoder, echo, reverb, chorus, flanger, phaser, vibrato, tremolo, ring modulator, distortion, bit crusher, filters, compressor, noise gate, noise, background sounds, radio beeps and glitches. Effects run from top to bottom. With Hear myself on, you hear every change right away.

### Shortcuts

In **Settings**, **Set up shortcuts** gives you keys for the voice changer, hear myself, mute, background, random, next and previous voice and stopping sounds. Every voice and every sound can get its own keys too. They work everywhere, even in games. Your desktop asks once to confirm them.

Desktops without global shortcuts can run commands instead:

| Command | What it does |
|---|---|
| `voicenux voice robot` | Use a voice. `voicenux list` shows them all. |
| `voicenux toggle`, `on`, `off` | Voice changer on or off |
| `voicenux hear` | Hear myself on or off |
| `voicenux mute` | Mute or unmute the microphone |
| `voicenux background` | Background sounds on or off |
| `voicenux random`, `next`, `previous` | Pick another voice |
| `voicenux sound airhorn` | Play a sound. `voicenux sounds` shows them all. |
| `voicenux stop` | Stop all sounds |
| `voicenux open soundboard` | Open the window on a page |

## Voices

<details>
<summary>All 68 voices</summary>

| Category | Voices |
|---|---|
| Characters | Baby, Kid, Girl, Deep Voice, Elderly, Granny, Titan, Troll, Santa, Pirate, Wizard, Fairy, Genie, Hero, Announcer, Movie Trailer, Noir Detective |
| Horror | Demon, Monster, Zombie, Ghost, Vampire, Witch, Dragon, Goblin, Possessed, Dark Lord, Creepy Clown |
| Sci-fi | Robot, Alien, Cyborg, Invader, Astronaut, Glitch, Hologram |
| Places | Echo, Cave, Cathedral, Shower, Stadium, Underwater, Deep Space, Storm, Campfire, Forest |
| Devices | Radio, Telephone, Walkie-Talkie, Megaphone, Pilot, 1920s Film, Potato Mic, 8-Bit, Fan |
| Music | Auto-Tune, Angel Choir, Magic Chords, Talk Box, Duet |
| Funny | Chipmunk, Mouse, Helium, Drunk, Dizzy, Whisper, Kitty, Frog, Duck |

Many of them come with a background sound: rain, bubbles, a campfire, a stadium crowd, wind, a heartbeat and more. All background sounds are made in code. The starter soundboard uses CC0 recordings from Freesound.

</details>

## Credits

The pictures are [Fluent Emoji](https://github.com/microsoft/fluentui-emoji) by Microsoft (MIT), their German names come from [Unicode CLDR](https://cldr.unicode.org). The reverb is Freeverb by Jezar at Dreampoint. Noise suppression is RNNoise by Xiph.Org, through [nnnoiseless](https://github.com/jneem/nnnoiseless). Audio files are read with [Symphonia](https://github.com/pdeljanov/Symphonia).

BSD-3-Clause.
