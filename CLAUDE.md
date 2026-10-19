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
- AI voices (`src/ai`, `src/dsp/ai.rs`, `data/ai/helper.py`) run RVC models in a Python helper,
  installed with one click into `~/.local/share/voicenux/ai` (uv, Python 3.11, PyTorch for CUDA
  12.6). The `ai` effect only moves samples through lock-free rings; the bridge thread talks to
  the helper over stdin and stdout. They want an NVIDIA card, or an AMD one: `ai::card` reads the
  chips from `/sys/class/kfd` and picks PyTorch for ROCm 7.2 (RX 6000 on; some pass as a close
  relative with `HSA_OVERRIDE_GFX_VERSION`). AMD is untested on real hardware. The setting
  "Run on the processor" (`ai_cpu`, or `VOICENUX_AI_CPU=1` for tests) installs a smaller PyTorch
  and runs them on the processor with bigger blocks and more delay. The helper warms a model up when it loads and
  picks the block size from how fast it renders. Blocks under 120 ms, or HuBERT without the
  mirrored window ends, make a held "aaa" jump between vowels. Models are pinned to a Hugging Face commit in
  `ai::MODELS`, with the character's typical pitch (the helper moves yours there, `key` only
  shifts from it) and whether to use the index. Only fictional characters, no real people as
  themselves.
- `src/api.rs` lets other apps control Voicenux (WebSocket and HTTP on 127.0.0.1:41967, off by
  default). Keep `API.md` in step with it. Raise `VERSION` there only for changes that break clients.
- Users never touch a config file. Every option has a place in the app, `src/config.rs` only
  stores it.
- Everything a user can read comes in English and German. Write the English text in the code and
  wrap it in `t()` (from `src/i18n.rs`), then add the German text to `po/de.po`. Names from data
  (voices, effects, sounds) go through `t_dyn()`. `make check` fails when a German text is missing.
  Use "du" in German.
- Pictures are Fluent Emoji 3D (MIT) in `data/icons`. Add new ones to `scripts/fetch-icons.sh` and
  run `make icons`. Never draw own pictures for voices or sounds. The AI voices have Fluent
  pictures in the colors of the character, made by `scripts/character-icons.py`. All other emoji
  are in `data/emoji`, small, for the picture picker; `data/emoji.tsv` has their English and German
  names for the search. `make emoji` makes both again; run it after `make icons`.
- The sidebar, the settings and the list of keys use Lucide icons (ISC) instead: plain lines in
  the text color, made by `ui::line_icon`. Colored tiles behind them were tried and dropped. They
  are paths in `data/lucide/icons.tsv`, drawn by `ui/glyph.rs`. Add new ones to `NAMES` in
  `scripts/fetch-lucide.py` and run `make lucide`.
- Look: colorful and Voicemod-like is wanted: the Fluent Emoji 3D pictures, the blue gradient
  (deep blue to cyan), glow and lively animations. A flat libadwaita look, a game-icons look and
  a violet to pink gradient were tried and dropped again. No wiggle animations. Do not redesign
  without being asked.
- GTK 4.24 picks the curve of a CSS transition by property, not by its place in the list. So use
  one curve per `transition` list, and springy curves only for transform alone. Otherwise a
  shadow can overshoot to a negative blur and GTK crashes.
- Rings and glows of tiles that move (voice cards, sound pads) stay inside the tile: use `inset`
  shadows. Shadows that reach out of a moving tile left broken bits on NVIDIA at 130 % scale.
- Header bars keep their middle free, so the window can be dragged there. Page tools like the
  search (`ui::search_field`) sit at the start, in line with the page below; a tool is a button
  with a label, not a bare icon, where there is room.
- Every popover goes through `popup::menu`: it bounces open, and a click next to it still does
  what it hits, like on other desktops. Right-click menus use `popup::context`. A click next to a
  dialog closes it; a dialog that would lose something sets `can_close` to false or `vn-stay`.
- Background sounds are real recordings: CC0 loops from Freesound in `data/ambience`, built into the
  app and listed in its `SOURCES.md`. `scripts/ambience.sh` cuts and levels them; add new ones
  there. Sounds made in code were tried and dropped: they sounded fake. Starter sounds come from
  MyInstants, as I asked: `src/soundboard/starter.rs` only has their links. Voicenux downloads them
  on the first start that is online and levels them when it loads them. Never put such clips in the
  repo. A new starter pack raises `PACK`, so older soundboards get the new sounds. **Find sounds**
  (`src/soundboard/online.rs`) searches MyInstants too. It reads the search page, and the page of
  each sound for its likes, since MyInstants has no API.

## Tests

- `make check` runs the unit tests: every effect, every voice (level and NaN), the German texts.
- `make e2e` runs `tests/e2e/run.py` against the real PipeWire graph: a fake microphone, test
  tones, and checks of what comes out of the Voicenux Microphone. Quit Voicenux first.
- `voicenux --headless <voice> [mic]` runs only the engine, good for poking at the graph with
  `pw-link -l` and `pw-record --target voicenux_mic`.
- Never test with injected input (XTest, ydotool, remote desktop portal). Drive the app with its
  own commands instead: `voicenux open <page>`, `voicenux voice <id>`, `voicenux sound <id>`.
- `voicenux --dialog keys|about|trim:SOUND|find:WORDS|tour:PAGE:STEP` opens a dialog, the
  MyInstants search or the tips of a page from one step on, for screenshots. Menus are popups of
  their own, `--snapshot` does not see them.
- For screenshots, `voicenux --snapshot file.png` renders the window into a PNG. It needs a desktop
  that draws frames: on a locked screen, run Voicenux in `kwin_wayland --virtual` with its own
  `dbus-daemon` and `GTK_A11Y=none`. `LANGUAGE=en` for English. A fake microphone fed by `pw-play`
  makes the meters move. Round the corners and add a shadow with ImageMagick for the README.

## Releases

Releases look like the ones of face-unlock and big projects such as Immich.
`.github/release-notes.sh <tag>` builds the notes: the entry from `CHANGELOG.md` (welcome and
highlights), a support section, every commit since the last release sorted by its prefix with
author and link, and the full changelog link. So commit subjects end up in public: keep them clear.
The `release` workflow builds packages for Debian 13, Ubuntu 26.04, Fedora and Arch
(`packaging/`, see its README), tries each one, and attaches them to the release.

1. Read `git log <last tag>..HEAD` and pick the version: only fixes → patch, something new → minor,
   something that breaks or needs the user to act → major.
2. Take the README screenshots again of every screen that changed since the last tag, and add
   ones for new UI (`docs: new screenshots`).
3. Bump `version` in `Cargo.toml`, add the release to `data/io.github.voicenux.Voicenux.metainfo.xml`
   and the entry at the top of `CHANGELOG.md`, in one commit (`chore: 0.2.0`).
4. Push, then push the tag: `git tag v0.2.0 && git push origin main v0.2.0`. The `release` workflow
   stops before building when the tag and `Cargo.toml` disagree or the entry is missing.

A minor or major release (has `### Highlights`, gets a heading and the support section):

```markdown
## v0.2.0

_2026-11-02_

Welcome to Voicenux `v0.2.0`! One or two sentences on what this release is about.

<p align="center">
  <img width="720" alt="What the picture shows" src="https://raw.githubusercontent.com/voicenux/voicenux/v0.2.0/res/<file>.png">
</p>

### 🚨 Breaking changes

- Only if there are any: what changed, and what the user has to do.

### Highlights

- First highlight, a few words
```

The picture is optional: a README screenshot of what is new. The highlights stay a plain list.
A patch release is just a sentence or two, for example: "A small patch. Find sounds loads again."
The list of commits follows on its own.

- Write for users: what they notice, not how the code does it. Friendly and simple.
- Commands they type go in backticks, buttons and labels in bold.
- To change an old release: edit its entry, commit, then
  `gh release edit <tag> --title <tag> --notes "$(.github/release-notes.sh <tag>)"`.

## README

- Keep it short, like face-unlock: install as one-liners per distribution (`yay -S voicenux-bin`,
  the apt and dnf repositories), three steps to use it, and the rest in `<details>` blocks with a
  sentence or two and their screenshot.
- New UI (a page, a dialog) gets a screenshot in the README, next to the text about it. A real
  screenshot of it running, in English, in dark mode.
- Screenshots are only made right before a release (see Releases), not after each change. So a
  release never shows an old one.
- Keep the voice list in the README in step with `data/voices.toml`.
