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

Send an action. `id` is optional and comes back with the answer:

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
| `favorite` | voice | Stars a voice, or takes the star away |
| `sound` | sound | Plays a sound, as the play mode of the soundboard says (from the start, play and stop, on top) |
| `sound-restart`, `sound-toggle`, `sound-overlap` | sound | Plays a sound from the start, plays or stops it, or plays another copy on top, whatever the soundboard says |
| `stop-sound` | sound | Stops a sound |
| `stop-sounds` | | Stops all sounds |
| `sound-volume` | 0 to 100 | Volume of the soundboard |
| `hear-sounds` | switch | Hear the soundboard yourself |
| `icon` | picture | WebSocket only: answers `{"png": "…base64…"}`. Over HTTP use `GET /api/icon/NAME` |

Actions that change something answer with the new state.

## Data

The state:

```json
{
  "voice": {"id": "robot", "name": "Robot", "icon": "robot", "category": "scifi"},
  "voice_changer": true,
  "hear_myself": false,
  "background": true,
  "mute": false,
  "sound_volume": 80,
  "hear_sounds": true,
  "playing": ["airhorn"],
  "apps": ["Discord"]
}
```

`apps` are the apps that hear your voice right now.

A voice: `{"id": "robot", "name": "Robot", "icon": "robot", "category": "scifi", "favorite": false, "custom": false}`. Categories are `characters`, `horror`, `scifi`, `places`, `devices`, `music`, `funny` and `custom` for voices made in Voicelab.

A sound: `{"id": "airhorn", "name": "Air Horn", "icon": "postal-horn", "mode": "restart", "volume": 100}`. `mode` is how a click plays it, the same for all sounds: `restart`, `toggle` or `overlap`.

Names come in the language of the app. `api` in the greeting goes up when a change breaks clients.
