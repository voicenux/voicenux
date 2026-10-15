//! Finds sounds on MyInstants and adds them with one click.
//! Listening plays them in the headphones only.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use super::{clear, label, smooth_scroll};
use crate::i18n::{fill, t};
use crate::soundboard::online::{self, Found};
use crate::soundboard::{RATE, SoundData, decode_bytes};
use crate::state::{Change, St};

/// A downloaded sound: the file to keep and the audio to play.
type Audio = Rc<(Vec<u8>, Arc<SoundData>)>;

struct Finder {
    st: St,
    list: gtk::ListBox,
    stack: gtk::Stack,
    message: gtk::Label,
    last: RefCell<String>,
    /// Counts searches, so an answer that comes late is dropped.
    search_no: Cell<u32>,
    results: RefCell<HashMap<String, Vec<Found>>>,
    audio: RefCell<HashMap<String, Audio>>,
    /// The play button of the sound you hear, and which listen it is.
    playing: RefCell<Option<gtk::Button>>,
    listen_no: Cell<u32>,
}

/// The panel, and its search field to focus.
pub fn panel(st: &St) -> (gtk::Box, gtk::SearchEntry) {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(t("Search MyInstants")));
    // Searches while typing, after a short pause. Enter searches at once.
    search.set_search_delay(250);
    search.set_margin_top(12);
    search.set_margin_start(12);
    search.set_margin_end(12);
    search.set_margin_bottom(6);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("vn-find-list");
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
    smooth_scroll(&scroll);

    let hint = label(
        t("Search sounds on MyInstants, listen, and add one with a click."),
        &["dim-label"],
    );
    let message = label("", &["dim-label"]);
    for l in [&hint, &message] {
        l.set_wrap(true);
        l.set_justify(gtk::Justification::Center);
        l.set_margin_start(24);
        l.set_margin_end(24);
    }
    let spinner = adw::Spinner::new();
    spinner.set_size_request(32, 32);
    let searching = label(t("Searching MyInstants…"), &["dim-label"]);
    let loading = gtk::Box::new(gtk::Orientation::Vertical, 12);
    loading.set_valign(gtk::Align::Center);
    loading.append(&spinner);
    loading.append(&searching);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vexpand(true);
    stack.add_named(&hint, Some("hint"));
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&scroll, Some("results"));
    stack.add_named(&message, Some("message"));

    let credit = label(t("Sounds from MyInstants"), &["dim-label", "caption"]);
    credit.set_margin_top(6);
    credit.set_margin_bottom(10);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("vn-detail");
    panel.add_css_class("vn-find");
    panel.set_width_request(340);
    panel.append(&search);
    panel.append(&stack);
    panel.append(&credit);

    let finder = Rc::new(Finder {
        st: st.clone(),
        list,
        stack,
        message,
        last: RefCell::default(),
        search_no: Cell::new(0),
        results: RefCell::default(),
        audio: RefCell::default(),
        playing: RefCell::default(),
        listen_no: Cell::new(0),
    });
    search.connect_search_changed({
        let finder = finder.clone();
        move |s| finder.search(&s.text())
    });
    search.connect_activate(move |s| finder.search(&s.text()));
    (panel, search)
}

impl Finder {
    fn search(self: &Rc<Self>, text: &str) {
        let query = text.trim().to_string();
        if *self.last.borrow() == query {
            return;
        }
        self.last.replace(query.clone());
        let no = self.search_no.get() + 1;
        self.search_no.set(no);
        if query.chars().count() < 2 {
            self.stack.set_visible_child_name("hint");
            return;
        }
        if let Some(found) = self.results.borrow().get(&query).cloned() {
            self.show(found);
            return;
        }
        self.stack.set_visible_child_name("loading");
        let me = self.clone();
        let q = query.clone();
        off_thread(
            move || online::search(&q),
            move |r| {
                if me.search_no.get() != no {
                    return;
                }
                match r {
                    Ok(found) => {
                        me.results.borrow_mut().insert(query, found.clone());
                        me.show(found);
                    }
                    Err(e) => {
                        // So the same words can be tried again.
                        me.last.replace(String::new());
                        me.say(error_text(e));
                    }
                }
            },
        );
    }

    fn say(&self, text: &str) {
        self.message.set_label(text);
        self.stack.set_visible_child_name("message");
    }

    fn show(self: &Rc<Self>, found: Vec<Found>) {
        clear(&self.list);
        if found.is_empty() {
            self.say(t("No sounds found. Try other words."));
            return;
        }
        for f in found {
            self.list.append(&self.row(f));
        }
        self.stack.set_visible_child_name("results");
    }

    fn row(self: &Rc<Self>, f: Found) -> gtk::ListBoxRow {
        let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
        play.add_css_class("flat");
        play.add_css_class("circular");
        play.set_valign(gtk::Align::Center);
        play.set_tooltip_text(Some(t("Listen")));

        let title = label(&f.title, &["vn-find-title"]);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_tooltip_text(Some(&f.title));

        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("suggested-action");
        add.add_css_class("circular");
        add.set_valign(gtk::Align::Center);
        add.set_tooltip_text(Some(t("Add to soundboard")));

        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        b.add_css_class("vn-find-row");
        b.append(&play);
        b.append(&title);
        b.append(&add);
        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_child(Some(&b));

        let f = Rc::new(f);
        play.connect_clicked({
            let (me, f) = (self.clone(), f.clone());
            move |b| me.listen(&f, b)
        });
        add.connect_clicked({
            let me = self.clone();
            move |b| me.add(&f, b)
        });
        row
    }

    /// Plays a sound in the headphones. A second click stops it.
    fn listen(self: &Rc<Self>, f: &Rc<Found>, button: &gtk::Button) {
        // Every click counts, so a stop also cancels a sound that is still loading.
        let no = self.listen_no.get() + 1;
        self.listen_no.set(no);
        let was = self.playing.take();
        if let Some(old) = &was {
            old.set_icon_name("media-playback-start-symbolic");
            self.st.stop_preview();
        }
        if was.as_ref() == Some(button) {
            return;
        }
        self.playing.replace(Some(button.clone()));
        let me = self.clone();
        self.with_audio(f, button, "media-playback-start-symbolic", move |button, audio| {
            // Another sound was started while this one was loading.
            if me.listen_no.get() != no {
                return;
            }
            me.st.preview(audio.1.clone());
            button.set_icon_name("media-playback-stop-symbolic");
            let seconds = audio.1.frames.len() as f32 / RATE as f32;
            let button = button.clone();
            glib::timeout_add_local_once(Duration::from_secs_f32(seconds + 0.2), move || {
                if me.listen_no.get() == no && me.playing.borrow().as_ref() == Some(&button) {
                    me.playing.take();
                    button.set_icon_name("media-playback-start-symbolic");
                }
            });
        });
    }

    fn add(self: &Rc<Self>, f: &Rc<Found>, button: &gtk::Button) {
        if button.has_css_class("vn-added") {
            return;
        }
        let me = self.clone();
        let f2 = f.clone();
        self.with_audio(f, button, "list-add-symbolic", move |button, audio| {
            let icon = super::emoji::guess(&f2.title).unwrap_or("loudspeaker");
            match me.st.add_found(&f2.title, &f2.ext, &audio.0, icon) {
                Ok(()) => {
                    button.set_icon_name("object-select-symbolic");
                    button.add_css_class("vn-added");
                    button.set_tooltip_text(Some(t("Added")));
                    me.st.notify(Change::Notice(fill(t("{} is on your soundboard now"), &[&f2.title])));
                }
                Err(e) => {
                    log::warn!("could not add {}: {e}", f2.title);
                    me.st.notify(Change::Notice(fill(t("Could not add {}"), &[&f2.title])));
                }
            }
        });
    }

    /// Gets the audio, from memory or by downloading it, while the button spins. `icon` is the
    /// icon of the button for after the spinning.
    fn with_audio(
        self: &Rc<Self>,
        f: &Rc<Found>,
        button: &gtk::Button,
        icon: &'static str,
        then: impl FnOnce(&gtk::Button, Audio) + 'static,
    ) {
        if let Some(audio) = self.audio.borrow().get(&f.url).cloned() {
            then(button, audio);
            return;
        }
        if button.child().is_some_and(|c| c.is::<adw::Spinner>()) {
            return;
        }
        button.set_child(Some(&adw::Spinner::new()));
        let found = (**f).clone();
        let (me, button) = (self.clone(), button.clone());
        off_thread(
            move || {
                let bytes = online::download(&found)?;
                let data = decode_bytes(bytes.clone(), &found.ext).map_err(|_| online::Error::Failed)?;
                Ok::<_, online::Error>((found, bytes, Arc::new(data)))
            },
            move |r| {
                button.set_icon_name(icon);
                match r {
                    Ok((found, bytes, data)) => {
                        let audio: Audio = Rc::new((bytes, data));
                        me.audio.borrow_mut().insert(found.url, audio.clone());
                        then(&button, audio);
                    }
                    Err(e) => me.st.notify(Change::Notice(error_text(e).to_string())),
                }
            },
        );
    }
}

fn error_text(e: online::Error) -> &'static str {
    match e {
        online::Error::Offline => t("Could not reach the sound search. Are you online?"),
        online::Error::TooMany => t("Too many searches. Wait a minute and try again."),
        online::Error::Failed => t("That did not work. Try again."),
    }
}

/// Runs `work` on its own thread and hands the result to `done` on the main thread.
/// When the work crashes, `done` still gets an answer, so no spinner turns forever.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, online::Error> + Send + 'static,
    done: impl FnOnce(Result<T, online::Error>) + 'static,
) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        done(rx.recv().await.unwrap_or(Err(online::Error::Failed)));
    });
}
