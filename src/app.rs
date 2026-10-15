//! The application: one instance, a window on demand, tray, shortcuts and the command line.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::i18n::{fill, t, t_dyn};
use crate::state::{Change, St, State};
use crate::ui::window::Window;
use crate::{shortcuts, tray};

pub const APP_ID: &str = "io.github.voicenux.Voicenux";

pub struct Ctx {
    pub st: St,
    app: adw::Application,
    window: RefCell<Option<Window>>,
    tray: RefCell<Option<tray::Handle>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    welcomed: std::cell::Cell<bool>,
}

pub fn usage() -> String {
    [
        "Usage: voicenux [COMMAND]",
        "",
        "Without a command, Voicenux opens its window.",
        "",
        "Commands:",
        "  --background      Start without a window (for autostart)",
        "  voice NAME        Use a voice (see `voicenux list`)",
        "  on | off | toggle Turn the voice changer on or off",
        "  hear              Hear myself on or off",
        "  background        Background sounds on or off",
        "  mute              Mute or unmute the microphone",
        "  random            Pick a random voice",
        "  next | previous   Next or previous voice",
        "  sound NAME        Play a sound from the soundboard",
        "  stop              Stop all sounds",
        "  list              List all voices",
        "  sounds            List all sounds",
        "  open PAGE         Open the window on voices, soundboard, voicelab or settings",
        "  --version         Show the version",
    ]
    .join("\n")
}

impl Ctx {
    fn window(self: &Rc<Self>) -> Window {
        if let Some(w) = self.window.borrow().clone() {
            return w;
        }
        let w = crate::ui::window::build(&self.app, &self.st);
        *self.window.borrow_mut() = Some(w.clone());
        w
    }

    pub fn show(self: &Rc<Self>) {
        let w = self.window();
        w.window.present();
        if !self.st.config.borrow().onboarded && !self.welcomed.replace(true) {
            crate::ui::onboarding::show(&self.st, &w.window);
        }
    }

    fn window_visible(&self) -> bool {
        self.window.borrow().as_ref().is_some_and(|w| w.window.is_visible())
    }

    /// Runs a named action from a shortcut, the tray or the command line.
    pub fn run(self: &Rc<Self>, id: &str) {
        let st = &self.st;
        let note = |text: String| st.notify(Change::Notice(text));
        match id {
            "voice-toggle" => {
                st.set_voice_on(!st.config.borrow().voice_on);
                note(if st.config.borrow().voice_on { t("Voice changer on") } else { t("Voice changer off") }.into());
            }
            "hear-toggle" => {
                st.set_hear_myself(!st.config.borrow().hear_myself);
                note(if st.config.borrow().hear_myself { t("Hear myself on") } else { t("Hear myself off") }.into());
            }
            "background-toggle" => {
                st.set_background(!st.config.borrow().background);
                note(if st.config.borrow().background { t("Background on") } else { t("Background off") }.into());
            }
            "mute-toggle" => {
                st.set_muted(!st.muted.get());
                note(if st.muted.get() { t("Microphone muted") } else { t("Microphone on") }.into());
            }
            "random-voice" => {
                st.random_voice();
                self.voice_note();
            }
            "next-voice" => {
                st.step_voice(true);
                self.voice_note();
            }
            "previous-voice" => {
                st.step_voice(false);
                self.voice_note();
            }
            "stop-sounds" => st.stop_sounds(),
            other => {
                if let Some(v) = other.strip_prefix("voice:") {
                    st.set_voice(v);
                    self.voice_note();
                } else if let Some(s) = other.strip_prefix("sound:") {
                    st.play_sound(s);
                }
            }
        }
    }

    fn voice_note(self: &Rc<Self>) {
        let name = t_dyn(&self.st.current().name);
        if self.window_visible() {
            self.st.notify(Change::Notice(fill(t("Voice: {}"), &[&name])));
        } else {
            let n = gio::Notification::new("Voicenux");
            n.set_body(Some(&fill(t("Voice: {}"), &[&name])));
            n.set_priority(gio::NotificationPriority::Low);
            self.app.send_notification(Some("voice"), &n);
        }
    }

    fn snapshot(&self) -> tray::Snapshot {
        let c = self.st.config.borrow();
        let voices = self.st.voices.borrow();
        tray::Snapshot {
            voice_on: c.voice_on,
            hear: c.hear_myself,
            muted: self.st.muted.get(),
            background: c.background,
            voice: voices.iter().find(|v| v.id == c.voice).map(|v| t_dyn(&v.name)).unwrap_or_default(),
            favorites: c
                .favorites
                .iter()
                .filter_map(|f| voices.iter().find(|v| &v.id == f).map(|v| (v.id.clone(), t_dyn(&v.name))))
                .collect(),
        }
    }

    fn keep_running(&self) {
        let want = self.st.runs_in_tray();
        let mut hold = self.hold.borrow_mut();
        if want && hold.is_none() {
            *hold = Some(self.app.hold());
        } else if !want {
            hold.take();
        }
    }

    fn command(self: &Rc<Self>, args: &[String], cmd: &gio::ApplicationCommandLine) -> i32 {
        let st = &self.st;
        let arg = |i: usize| args.get(i).map(String::as_str);
        match arg(0) {
            None => self.show(),
            Some("--background") => {}
            Some("open") => {
                self.show();
                let page = arg(1).unwrap_or("voices");
                if let Some(w) = self.window.borrow().as_ref() {
                    w.show_page(page);
                }
            }
            // For README screenshots: renders the window into a PNG. Not in the help on purpose.
            Some("--snapshot") => {
                let Some(path) = arg(1) else { return 2 };
                let Some(w) = self.window.borrow().clone() else { return 1 };
                if let Err(e) = crate::ui::snapshot(w.window.upcast_ref(), path) {
                    cmd.printerr_literal(&format!("{e}\n"));
                    return 1;
                }
            }
            // Also for screenshots: opens a dialog, "keys", "about" or "trim:SOUND", or searches
            // MyInstants with "find:WORDS".
            Some("--dialog") => {
                self.show();
                let Some(w) = self.window.borrow().clone() else { return 1 };
                match arg(1) {
                    Some("keys") => crate::ui::help::show(&w.window),
                    Some("about") => crate::ui::settings::about_dialog().present(Some(&w.window)),
                    Some(other) if other.starts_with("trim:") => crate::ui::trim::open(st, &other[5..], &w.window),
                    Some(other) if other.starts_with("find:") => {
                        w.show_page("soundboard");
                        (w.find_online)(&other[5..]);
                    }
                    _ => return 2,
                }
            }
            Some("on") => st.set_voice_on(true),
            Some("off") => st.set_voice_on(false),
            Some("toggle") => self.run("voice-toggle"),
            Some("hear") => self.run("hear-toggle"),
            Some("background") => self.run("background-toggle"),
            Some("mute") => self.run("mute-toggle"),
            Some("random") => self.run("random-voice"),
            Some("next") => self.run("next-voice"),
            Some("previous") => self.run("previous-voice"),
            Some("stop") => self.run("stop-sounds"),
            Some("voice") => {
                let name = args[1..].join(" ").to_lowercase();
                let found = st
                    .voices
                    .borrow()
                    .iter()
                    .find(|v| v.id == name || v.name.to_lowercase() == name || t_dyn(&v.name).to_lowercase() == name)
                    .map(|v| v.id.clone());
                match found {
                    Some(id) => self.run(&format!("voice:{id}")),
                    None => {
                        cmd.printerr_literal(&format!("No voice called \"{name}\". See `voicenux list`.\n"));
                        return 1;
                    }
                }
            }
            Some("sound") => {
                let name = args[1..].join(" ").to_lowercase();
                let found = st
                    .library
                    .borrow()
                    .sound
                    .iter()
                    .find(|s| s.id == name || s.name.to_lowercase() == name || t_dyn(&s.name).to_lowercase() == name)
                    .map(|s| s.id.clone());
                match found {
                    Some(id) => st.play_sound(&id),
                    None => {
                        cmd.printerr_literal(&format!("No sound called \"{name}\". See `voicenux sounds`.\n"));
                        return 1;
                    }
                }
            }
            Some(other) => {
                cmd.printerr_literal(&format!("Unknown command \"{other}\".\n\n{}\n", usage()));
                return 2;
            }
        }
        0
    }
}

pub fn run() -> glib::ExitCode {
    gio::resources_register_include!("voicenux.gresource").expect("resources are built in");
    let app =
        adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    let ctx: Rc<RefCell<Option<Rc<Ctx>>>> = Rc::default();

    app.connect_startup({
        let ctx = ctx.clone();
        move |app| {
            glib::set_application_name("Voicenux");
            gtk::Window::set_default_icon_name(APP_ID);
            let provider = gtk::CssProvider::new();
            provider.load_from_string(&crate::ui::delay_css());
            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().expect("a display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap())
                .add_resource_path(&format!("{}/hicolor", crate::ui::RESOURCE));
            // The sidebar shows the logo already, so the window icon in the title buttons goes.
            let settings = gtk::Settings::default().expect("a display");
            drop_window_icon(&settings);
            settings.connect_gtk_decoration_layout_notify(drop_window_icon);

            let st = State::new();
            apply_appearance(&st);
            crate::api::start(&st);
            let c = Rc::new(Ctx {
                st: st.clone(),
                app: app.clone(),
                window: RefCell::default(),
                tray: RefCell::default(),
                hold: RefCell::default(),
                welcomed: Default::default(),
            });
            // Tray.
            let (tx, rx) = async_channel::unbounded();
            *c.tray.borrow_mut() = tray::spawn(tx, c.snapshot());
            st.tray.set(c.tray.borrow().is_some());
            c.keep_running();
            glib::spawn_future_local({
                let c = c.clone();
                async move {
                    while let Ok(cmd) = rx.recv().await {
                        match cmd {
                            tray::Cmd::Show => c.show(),
                            tray::Cmd::Action(a) => c.run(a),
                            tray::Cmd::Voice(v) => c.run(&format!("voice:{v}")),
                            tray::Cmd::Quit => c.app.quit(),
                        }
                    }
                }
            });

            // Shortcuts.
            let runner: Rc<dyn Fn(&str)> = Rc::new({
                let weak = Rc::downgrade(&c);
                move |id: &str| {
                    if let Some(c) = weak.upgrade() {
                        c.run(id);
                    }
                }
            });
            shortcuts::start(&st, &st.keys, runner);

            st.subscribe({
                let weak = Rc::downgrade(&c);
                move |change| {
                    let Some(c) = weak.upgrade() else { return };
                    if matches!(
                        change,
                        Change::Toggles | Change::Voice | Change::Favorites | Change::Voices | Change::VoiceEdited(_)
                    ) && let Some(h) = c.tray.borrow().as_ref()
                    {
                        tray::update(h, c.snapshot());
                    }
                    if matches!(change, Change::Settings) {
                        c.keep_running();
                        apply_appearance(&c.st);
                    }
                }
            });
            // Logging out sends SIGTERM: save and leave like a normal quit.
            for sig in [libc_sigterm(), 2] {
                let app = app.clone();
                glib_unix::unix_signal_add_local(sig, move || {
                    app.quit();
                    glib::ControlFlow::Break
                });
            }
            *ctx.borrow_mut() = Some(c);
        }
    });

    app.connect_command_line({
        let ctx = ctx.clone();
        move |_, cmd| {
            let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a| a.to_string_lossy().to_string()).collect();
            let Some(c) = ctx.borrow().clone() else { return glib::ExitCode::FAILURE };
            glib::ExitCode::from(c.command(&args, cmd) as u8)
        }
    });

    app.connect_shutdown({
        let ctx = ctx.clone();
        move |_| {
            if let Some(c) = ctx.borrow_mut().take() {
                c.st.save_now();
                c.st.empty_trash();
            }
        }
    });

    app.run()
}

const fn libc_sigterm() -> i32 {
    15
}

pub fn apply_appearance(st: &St) {
    let scheme = match st.config.borrow().appearance.as_str() {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

/// Removes "icon" from the title button layout, like "icon:minimize,close" from KDE.
fn drop_window_icon(s: &gtk::Settings) {
    let Some(layout) = s.gtk_decoration_layout() else { return };
    let clean = layout
        .split(':')
        .map(|side| side.split(',').filter(|b| b.trim() != "icon").collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join(":");
    if clean != layout.as_str() {
        s.set_gtk_decoration_layout(Some(&clean));
    }
}

/// Writes or removes the autostart entry.
pub fn set_autostart(on: bool) -> std::io::Result<()> {
    let dir = dirs::config_dir().unwrap_or_default().join("autostart");
    let path = dir.join(format!("{APP_ID}.desktop"));
    if !on {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let exe = std::env::current_exe().ok();
    let exec = match &exe {
        Some(p) if !p.starts_with("/usr") => p.display().to_string(),
        _ => "voicenux".to_string(),
    };
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=Voicenux\nComment={}\nExec={exec} --background\nIcon={APP_ID}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
            t("Voice changer for Linux")
        ),
    )
}
