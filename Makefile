PREFIX ?= /usr/local
DESTDIR ?=
APP_ID = io.github.voicenux.Voicenux
BIN = target/release/voicenux

.PHONY: all build check e2e run install uninstall icons emoji lucide clean

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
	install -Dm644 data/app/$(APP_ID)-symbolic.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg
	install -Dm644 data/app/$(APP_ID)-background-symbolic.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-background-symbolic.svg
	install -Dm644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/voicenux/LICENSE
	install -Dm644 data/lucide/LICENSE $(DESTDIR)$(PREFIX)/share/licenses/voicenux/LICENSE.lucide

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/voicenux
	rm -f $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	rm -f $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-background-symbolic.svg
	rm -rf $(DESTDIR)$(PREFIX)/share/licenses/voicenux

# Downloads missing Fluent Emoji pictures listed in the script, and makes the pictures of the AI
# voices from them. Needs Pillow and NumPy.
icons:
	scripts/fetch-icons.sh
	python3 -I scripts/character-icons.py

# Downloads all other Fluent Emoji for the picture picker, small, and their names. Needs Pillow.
emoji:
	python3 -I scripts/fetch-emoji.py

# Downloads the Lucide icons of the sidebar and the settings, listed in the script.
lucide:
	python3 -I scripts/fetch-lucide.py

clean:
	cargo clean
