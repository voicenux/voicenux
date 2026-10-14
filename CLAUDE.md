# CLAUDE.md

## Style

- Keep everything short: replies, explanations, comments, docs.
- Use simple English: short sentences, common words.
- Avoid em dashes (—). Do not just swap them for "-" either. Rewrite the sentence instead, for
  example with a comma, a colon, brackets or two sentences.

## Commits

- English only.
- Conventional Commits prefix: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `ci:`, `build:`, `test:`.
- Subject as short as possible. Imperative, lowercase, no trailing period.
- Body only when something genuinely cannot be inferred from the diff.
- Never add `Co-Authored-By`, "Generated with" or any other AI attribution to commits or PR descriptions.
- Do not commit or push before I have checked the changes locally and said they are fine. Then
  commit and push. Several attempts at the same thing make one commit, and attempts that did not
  work make none. Different things done in one session get a commit each. This also goes for
  releases and tags.

## Project

- Rust with GTK 4 and libadwaita. App ID `io.github.voicenux.Voicenux`. Do not change it: the
  desktop file, the shortcut portal and the tray all hang on it.
- `src/dsp/` is the sound: every effect is an `Effect`. It runs on PipeWire's realtime thread, so
  `process` never allocates, locks, frees, logs or does file I/O. Allocate in `new`. New things
  reach the realtime side through `RtCmd`, old ones leave through `Trash` (`src/audio/rt.rs`).
- An effect is a `BlockSpec` in `src/dsp/blocks.rs`. The order of `params` is the index its
  `set_param` gets, so change both together. `.main()` puts a setting on the voice page; keep
  that to the 1 to 3 settings people understand. Everything shows in Voicelab anyway.
- Voices are in `data/voices.toml`. `Chain::build_leveled` measures every voice and brings it to
  the loudness of the dry voice, so do not add gain blocks to fix levels.
- `src/audio/graph.rs` owns the PipeWire connection: a `pw_filter` (`voicenux_engine`), the virtual
  source `voicenux_mic` and the links between them, the microphone and the headphones. pipewire-rs
  has no filter yet, `src/audio/filter.rs` wraps the C API.
- Users never touch a config file. Every option has a place in the app, `src/config.rs` only
  stores it.
- Everything a user can read comes in English and German. Write the English text in the code and
  wrap it in `t()` (from `src/i18n.rs`), then add the German text to `po/de.po`. Names from data
  (voices, effects, sounds) go through `t_dyn()`. `make check` fails when a German text is missing.
  Use "du" in German.
- Pictures are Fluent Emoji 3D (MIT) in `data/icons`. Add new ones to `scripts/fetch-icons.sh` and
  run `make icons`. Never draw own pictures for voices or sounds.
- Background sounds and starter sounds are made in code (`src/dsp/ambience.rs`,
  `src/soundboard/synth.rs`). Do not add sample files.

## Tests

- `make check` runs the unit tests: every effect, every voice (level and NaN), the German texts.
- `make e2e` runs `tests/e2e/run.py` against the real PipeWire graph: a fake microphone, test
  tones, and checks of what comes out of the Voicenux Microphone. Quit Voicenux first.
- `voicenux --headless <voice> [mic]` runs only the engine, good for poking at the graph with
  `pw-link -l` and `pw-record --target voicenux_mic`.
- Never test with injected input (XTest, ydotool, remote desktop portal). Drive the app with its
  own commands instead: `voicenux open <page>`, `voicenux voice <id>`, `voicenux sound <id>`.
- For screenshots, put the window on top with a KWin script and use `spectacle -b -n -a`.
  `LANGUAGE=en` for English. A fake microphone fed by `pw-play` makes the meters move.

## Releases

1. Read `git log <last tag>..HEAD` and pick the version: only fixes → patch, something new → minor,
   something that breaks or needs the user to act → major.
2. Bump `version` in `Cargo.toml`, add the release to `data/io.github.voicenux.Voicenux.metainfo.xml`
   and the entry at the top of `CHANGELOG.md`, in one commit (`chore: 0.2.0`).
3. Push, then push the tag: `git tag v0.2.0 && git push origin main v0.2.0`.

A minor or major release starts with a welcome sentence and has a `### Highlights` list. A patch
release is just a sentence or two. Write for users: what they notice, not how the code does it.

## README

- New UI (a page, a dialog) gets a screenshot in the README, next to the text about it. A real
  screenshot of it running, in English, in dark mode.
- When a screen changes, take its screenshot again. The README never shows an old one.
- Keep the voice list in the README in step with `data/voices.toml`.
