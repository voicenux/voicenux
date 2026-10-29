# Voicenux API

Other apps on your computer can control Voicenux: a plugin for [OpenDeck](https://github.com/nekename/OpenDeck) or a Stream Deck, a script, a bot. Turn it on in **Settings → Other apps**.

It listens on `127.0.0.1:41967` only, so nothing outside your computer reaches it. Requests from web pages are refused: a request with an `Origin` header must come from `localhost`, `127.0.0.1`, `file://` or `tauri://`. Programs send no `Origin`, so they are fine.

## Allowed apps

The first time an app connects, Voicenux asks the user whether it may control Voicenux. There is no password: Voicenux looks up the program on the other end of the connection and remembers it. Until the user answers, the connection waits, for up to two minutes. An app that is not allowed gets status 403, also for the WebSocket handshake. Try again later: the user may allow it then. The user can turn this off, and take apps back, in **Settings → Other apps**.

Pictures (`GET /api/icon/NAME`) need no answer.

## WebSocket

Connect to `ws://127.0.0.1:41967`. Voicenux greets you, then sends everything you need to draw your keys:

```json
{"event": "hello", "data": {"app": "voicenux", "version": "0.1.0", "api": 1}}
{"event": "state", "data": {…}}
{"event": "voices", "data": [ … ]}
{"event": "sounds", "data": [ … ]}
```

Send an action. `id` is optional and comes back with the answer. An `arg` can be text, a number, a switch as `true` or `false`, or an object for actions that take more than one thing:

```json
{"id": 1, "action": "voice", "arg": "robot"}
{"id": 1, "ok": true, "result": {…the new state…}}
{"id": 2, "action": "fly"}
{"id": 2, "ok": false, "error": "unknown action \"fly\", see API.md"}
```

Voicenux tells you about every change, also when someone uses the window, a shortcut or the tray:

| Event | When | `data` |
|---|---|---|
| `state` | the voice, a switch or the soundboard volume changes | the state, see below |
| `voices` | a voice is added, changed or starred | the list of voices |
| `sounds` | a sound is added, changed, moved or removed | the list of sounds |
| `playing` | a sound starts or stops | ids of the sounds that play |

## HTTP

The same actions, for tools that only send requests:

| Request | Does |
|---|---|
| `GET /api/state` | The state |
| `GET /api/voices` | All voices |
| `GET /api/sounds` | All sounds |
| `GET /api/icon/NAME` | The picture `NAME` of a voice or sound, as PNG |
| `POST /api/ACTION` | An action without `arg`, or with the body as `arg` (plain text, or JSON `{"arg": …}`) |
| `POST /api/ACTION/ARG` | An action with `arg` |

Actions only go with `POST`. Answers look like `{"ok": true, "result": …}`, or `{"ok": false, "error": "…"}` with status 400.

```bash
curl -X POST http://127.0.0.1:41967/api/sound/airhorn
curl -X POST http://127.0.0.1:41967/api/voice-changer/off
curl http://127.0.0.1:41967/api/icon/robot -o robot.png
```

## Actions

`arg` of a switch is `on`, `off` or `toggle`, and leaving it out toggles. Voices and sounds go by id, or by name in English or in the language of the app.

| Action | `arg` | Does |
|---|---|---|
| `state` | | Answers with the state |
| `voices` | | Answers with the list of voices |
| `sounds` | | Answers with the list of sounds |
| `voice` | voice | Uses a voice, and turns the voice changer on |
| `random-voice` | | Uses a random voice |
| `random-favorite` | | Uses a random voice out of your favorites |
| `next-voice`, `previous-voice` | | Uses the next or previous voice, out of your favorites when you have some |
| `voice-changer` | switch | Voice changer on or off |
| `hear-myself` | switch | Hear myself on or off |
| `background` | switch | Background sounds of the voice on or off |
| `mute` | switch | Mutes the microphone |
| `listen` | switch | Listens to the microphone, for 15 s at most. Others do not hear you meanwhile, sounds still play. `off` throws away what it heard |
| `transcribe` | language | Stops listening and answers with what you said: `{"text": "vine boom"}`, or `""`. Leave out the language (`en`, `de`, …) for English or the language of the app |
| `find-voice` | words | Answers with the voice that fits what someone said, like `{"id": "vader", "name": "Darth Vader", "icon": "…", "favorite": false}`, also when speech recognition heard it a little off. Only voices you have in Voicenux. It does not use the voice: send `voice` with its id for that |
| `find-sound` | words, or `{"words": "…", "rank": 2}` | Finds the sound most people liked on MyInstants, for a key of your own. `rank` 2 is the one with the second most likes, and so on; past the last it starts over. It stays off the soundboard: Voicenux keeps the last 50 of them, and drops the one used longest ago. A sound you have already is taken as it is, without a download: one with that name, or the same file on MyInstants. Answers `{"id": "found-3", "name": "Vine boom sound", "icon": "collision", "likes": 3276, "soundboard": false, "rank": 1, "of": 8}`. `likes` and `of` are `null` for a sound found by its name. Play it with the sound actions, by its id |
| `favorite` | voice | Stars a voice, or takes the star away |
| `sound` | sound | Plays a sound, as the play mode of the soundboard says (from the start, play and stop, on top) |
| `sound-restart`, `sound-toggle`, `sound-overlap` | sound | Plays a sound from the start, plays or stops it, or plays another copy on top, whatever the soundboard says |
| `stop-sound` | sound | Stops a sound |
| `stop-sounds` | | Stops all sounds |
| `sound-volume` | 0 to 100 | Volume of the soundboard |
| `hear-sounds` | switch | Hear the soundboard yourself |
| `icon` | picture | WebSocket only: answers `{"png": "…base64…"}`. Over HTTP use `GET /api/icon/NAME` |

Actions that change something answer with the new state.

`transcribe` and `find-sound` take a few seconds, or longer the first time: `transcribe` downloads the speech recognition then (Whisper base, about 290 MB). Over WebSocket other actions go on meanwhile, so their answers may come first.

## Data

The state:

```json
{
  "voice": {"id": "robot", "name": "Robot", "icon": "robot", "category": "scifi"},
  "voice_changer": true,
  "hear_myself": false,
  "background": true,
  "mute": false,
  "listening": false,
  "sound_volume": 80,
  "hear_sounds": true,
  "playing": ["airhorn"],
  "apps": ["Discord"]
}
```

`apps` are the apps that hear your voice right now. `listening` is true while an app listens, see `listen`.

A voice: `{"id": "robot", "name": "Robot", "icon": "robot", "category": "scifi", "favorite": false, "custom": false}`. Categories are `characters`, `horror`, `scifi`, `places`, `devices`, `music`, `funny` and `custom` for voices made in Voicelab.

A sound: `{"id": "airhorn", "name": "Air Horn", "icon": "postal-horn", "mode": "restart", "volume": 100, "favorite": false}`. `mode` is how a click plays it, the same for all sounds: `restart`, `toggle` or `overlap`. `favorite` is true for a sound with a star.

Names come in the language of the app. `api` in the greeting goes up when a change breaks clients.
