# Changelog

What each release brings, newest first. The [GitHub releases](https://github.com/voicenux/voicenux/releases) add every commit that went into it.

## v0.4.0

_2026-10-30_

Welcome to Voicenux `v0.4.0`! Voicenux now speaks seven more languages, and it understands what you say: search sounds by voice, or let a Stream Deck key find a sound for you.

<p align="center">
  <img width="720" alt="Find sounds next to the soundboard, with a microphone button next to the search field. It lists applause sounds from MyInstants with their likes and views" src="https://raw.githubusercontent.com/voicenux/voicenux/v0.4.0/res/soundboard.png">
</p>

### Highlights

- Voicenux in Spanish, French, Brazilian Portuguese, Italian, Polish, Russian and Turkish
- Say what you look for: the microphone button in **Find sounds** fills in the search
- Apps can find sounds and voices by what you say, like the new **Say a Sound** key of the Stream Deck plugin
- The speech recognition runs on your computer, nothing you say goes online. It downloads the first time you use it, and **Settings** can remove it
- A short sound when you turn the voice changer, **Hear myself**, the background or mute on or off. Only you hear it, and **Settings** has its volume
- Your EasyEffects microphone effects run before the voice without asking. Turn it off in **Advanced settings**

## v0.3.0

_2026-10-27_

Welcome to Voicenux `v0.3.0`! Voicenux now asks before another app may control it, and the soundboard tells you when Discord may filter out your sounds.

<p align="center">
  <img width="720" alt="Allow Voicenux Plugin to control Voicenux?: a dialog with the path of the plugin, Open folder, and Do not allow or Allow" src="https://raw.githubusercontent.com/voicenux/voicenux/v0.3.0/res/ask.png">
</p>

### Highlights

- Voicenux asks before an app, like the Stream Deck plugin or a script, may control it
- A note on the soundboard when Discord may filter out your sounds, with what to change in Discord
- The mouse wheel moves sliders, in round steps, and stops where the voice started out
- You hear a voice change while you drag its slider
- A random favorite: the arrow next to **Random voice**, or `Ctrl+Shift+R`
- Starred sounds come first on the soundboard
- **Advanced settings** for tinkerers, at the bottom of Settings

## v0.2.0

_2026-10-19_

Welcome to Voicenux `v0.2.0`! The AI voices now run on AMD graphics cards too, and the first start asks whether Voicenux should start with your computer. AMD support is brand new, so please tell us in an issue how it works on your card.

### Highlights

- AI voices on AMD graphics cards from the RX 6000 series on. Voicenux finds your card by itself
- The welcome screen asks about **Start with the computer**
- New API actions `sound-restart`, `sound-toggle` and `sound-overlap`, so a Stream Deck key can play a sound its own way

## v0.1.0

_2026-10-18_

Welcome to Voicenux `v0.1.0`, the voice changer for Linux! Change your voice in real time in Discord, games, OBS and any other app.

<p align="center">
  <img width="720" alt="Voicenux with the Baby voice selected: a grid of voice cards, the voice with its pitch settings on the right, and the switches for voice changer, hear myself and background at the bottom" src="https://raw.githubusercontent.com/voicenux/voicenux/v0.1.0/res/voices.png">
</p>

### Highlights

- 59 voices in seven categories, from Baby and Granny to Demon, Robot, Underwater and Auto-Tune
- 10 AI voices like SpongeBob, Darth Vader and GLaDOS, best with an NVIDIA graphics card
- Real recorded background sounds, like rain, a campfire, a stadium crowd or a thunderstorm
- The Voicenux Microphone for Discord, OBS, games and every other app, or as your default microphone
- Hear myself, mute, noise suppression, a noise gate and your EasyEffects before the voice
- A soundboard with starter sounds, folders and your own files: trim them, or find new ones on MyInstants
- Voicelab: build your own voices from 26 effects
- Folders, favorites and drag and drop for voices and sounds
- Global shortcuts, keys in the app, a tray icon, a command line and an API for the Stream Deck
- Tips on the first visit of each page
- English and German
- Packages for Debian 13, Ubuntu 26.04 and Fedora
