PREFIX ?= /usr/local
DESTDIR ?=
APP_ID = io.github.voicenux.Voicenux
BIN = target/release/voicenux

.PHONY: all build check e2e run install uninstall icons sounds clean

all: build

build:
	cargo build --release --locked

# Unit tests, including the check that every text has a German translation.
check:
	cargo test --release --locked

# Plays test tones through the real PipeWire graph. Quit Voicenux first.
e2e: build
	python3 tests/e2e/run.py $(BIN)

run:
	cargo run --release

install:
	@test -f $(BIN) || { echo "Run make first, then sudo make install."; exit 1; }
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/voicenux
	install -Dm644 data/$(APP_ID).desktop $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	install -Dm644 data/$(APP_ID).metainfo.xml $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	install -Dm644 data/app/$(APP_ID).svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	install -Dm644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/voicenux/LICENSE

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/voicenux
	rm -f $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	rm -f $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	rm -rf $(DESTDIR)$(PREFIX)/share/licenses/voicenux

# Downloads missing Fluent Emoji pictures listed in the script.
icons:
	scripts/fetch-icons.sh

# Downloads the CC0 starter sounds from Freesound and levels them. Needs ffmpeg.
sounds:
	scripts/fetch-sounds.py

clean:
	cargo clean
