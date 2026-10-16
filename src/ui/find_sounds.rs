//! Finds sounds on MyInstants and adds them with one click. Before a search it shows the popular
//! ones. Listening plays them in the headphones only.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use super::{clear, label, line_icon, smooth_scroll};
use crate::i18n::{fill, t};
use crate::soundboard::online::{self, Found, Stats};
use crate::soundboard::{RATE, SoundData, decode_bytes};
use crate::state::{Change, St};

/// A downloaded sound: the file to keep and the audio to play.
type Audio = Rc<(Vec<u8>, Arc<SoundData>)>;

/// The ways to sort, with their names in the config.
const SORTS: [(&str, &str); 5] =
    [("suggested", "Suggested"), ("views", "Most views"), ("likes", "Most likes"), ("newest", "Newest"), ("az", "A to Z")];

struct Finder {
    st: St,
    list: gtk::ListBox,
    stack: gtk::Stack,
    message: gtk::Label,
    heading: gtk::Label,
    last: RefCell<String>,
    /// Counts searches, so an answer that comes late is dropped.
    search_no: Cell<u32>,
    /// Found sounds by search words. The popular ones are under "".
    results: RefCell<HashMap<String, Vec<Found>>>,
    audio: RefCell<HashMap<String, Audio>>,
    /// Likes and views of the sounds by their file, once their pages are read.
    stats: RefCell<HashMap<String, Stats>>,
    /// The rows shown, with their place in the order MyInstants gave.
    rows: RefCell<HashMap<gtk::ListBoxRow, (usize, Rc<Found>)>>,
    /// An index into [`SORTS`].
    sort: Cell<usize>,
    resort_soon: Cell<bool>,
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

    let message = label("", &["dim-label"]);
    message.set_wrap(true);
    message.set_justify(gtk::Justification::Center);
    message.set_margin_start(24);
    message.set_margin_end(24);
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
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&scroll, Some("results"));
    stack.add_named(&message, Some("message"));

    let credit = label(t("Sounds from MyInstants"), &["dim-label", "caption"]);
    credit.set_margin_top(6);
    credit.set_margin_bottom(10);

    // What the list shows, and how it is sorted.
    let heading = label("", &["vn-find-heading"]);
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let names: Vec<&str> = SORTS.iter().map(|(_, name)| t(name)).collect();
    let sort = gtk::DropDown::from_strings(&names);
    sort.set_tooltip_text(Some(t("Sort")));
    sort.set_valign(gtk::Align::Center);
    let chosen = SORTS.iter().position(|(id, _)| *id == st.config.borrow().sound_sort).unwrap_or(0);
    sort.set_selected(chosen as u32);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("vn-find-bar");
    bar.append(&heading);
    bar.append(&sort);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("vn-detail");
    panel.add_css_class("vn-find");
    panel.set_width_request(340);
    panel.append(&search);
    panel.append(&bar);
    panel.append(&stack);
    panel.append(&credit);

    let finder = Rc::new(Finder {
        st: st.clone(),
        list,
        stack,
        message,
        heading,
        // Not a search anyone types, so the first one always runs.
        last: RefCell::new("\u{0}".into()),
        search_no: Cell::new(0),
        results: RefCell::default(),
        audio: RefCell::default(),
        stats: RefCell::default(),
        rows: RefCell::default(),
        sort: Cell::new(chosen),
        resort_soon: Cell::new(false),
        playing: RefCell::default(),
        listen_no: Cell::new(0),
    });
    finder.list.set_sort_func({
        let finder = Rc::downgrade(&finder);
        move |a, b| finder.upgrade().map_or(std::cmp::Ordering::Equal, |f| f.order(a, b)).into()
    });
    sort.connect_selected_notify({
        let finder = finder.clone();
        move |d| {
            let i = (d.selected() as usize).min(SORTS.len() - 1);
            finder.sort.set(i);
            finder.st.config.borrow_mut().sound_sort = SORTS[i].0.into();
            finder.st.save_later();
            finder.list.invalidate_sort();
        }
    });
    // The popular sounds load when the panel opens the first time, not when the app starts.
    panel.connect_map({
        let (finder, search) = (finder.clone(), search.clone());
        move |_| finder.search(&search.text())
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
        // Without a search: the popular sounds.
        let query = if query.chars().count() < 2 { String::new() } else { query };
        if let Some(found) = self.results.borrow().get(&query).cloned() {
            self.show(&query, found);
            return;
        }
        self.stack.set_visible_child_name("loading");
        let me = self.clone();
        let q = query.clone();
        off_thread(
            move || if q.is_empty() { online::popular() } else { online::search(&q) },
            move |r| {
                if me.search_no.get() != no {
                    return;
                }
                match r {
                    Ok(found) => {
                        me.results.borrow_mut().insert(query.clone(), found.clone());
                        me.show(&query, found);
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

    fn show(self: &Rc<Self>, query: &str, found: Vec<Found>) {
        clear(&self.list);
        self.rows.borrow_mut().clear();
        if found.is_empty() {
            self.heading.set_label("");
            self.say(t("No sounds found. Try other words."));
            return;
        }
        let heading = if query.is_empty() {
            t("Popular on MyInstants").to_string()
        } else {
            fill(t("{} sounds"), &[&found.len().to_string()])
        };
        self.heading.set_label(&heading);
        let mut waiting = VecDeque::new();
        for (i, f) in found.into_iter().enumerate() {
            let line = stats_line();
            let row = self.row(&f, &line);
            self.list.append(&row);
            let f = Rc::new(f);
            self.rows.borrow_mut().insert(row, (i, f.clone()));
            match self.stats.borrow().get(&f.url) {
                Some(s) => show_stats(&line, *s),
                None => waiting.push_back((f, line)),
            }
        }
        self.list.invalidate_sort();
        self.stack.set_visible_child_name("results");
        // Each sound's numbers are a page of their own: read a few at a time, top first.
        let waiting = Rc::new(RefCell::new(waiting));
        for _ in 0..3 {
            self.next_stats(self.search_no.get(), waiting.clone());
        }
    }

    /// Reads the numbers of the next sound in `waiting`, then goes on, until a new search starts.
    fn next_stats(self: &Rc<Self>, no: u32, waiting: Rc<RefCell<VecDeque<(Rc<Found>, gtk::Box)>>>) {
        if self.search_no.get() != no {
            return;
        }
        let Some((found, line)) = waiting.borrow_mut().pop_front() else { return };
        let me = self.clone();
        let sent = (*found).clone();
        off_thread(
            move || online::stats(&sent),
            move |r| match r {
                Ok(s) => {
                    me.stats.borrow_mut().insert(found.url.clone(), s);
                    show_stats(&line, s);
                    me.resort();
                    me.next_stats(no, waiting);
                }
                // MyInstants wants a break: the rest stays without numbers.
                Err(online::Error::TooMany) => waiting.borrow_mut().clear(),
                Err(_) => me.next_stats(no, waiting),
            },
        );
    }

    /// Sorts again for numbers that came in, at most a few times a second, so rows do not jump
    /// with every single one.
    fn resort(self: &Rc<Self>) {
        if !matches!(SORTS[self.sort.get()].0, "views" | "likes") || self.resort_soon.replace(true) {
            return;
        }
        let me = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            me.resort_soon.set(false);
            me.list.invalidate_sort();
        });
    }

    /// The order of two rows. Sounds without numbers yet go after those with, and the order
    /// MyInstants gave settles the rest.
    fn order(&self, a: &gtk::ListBoxRow, b: &gtk::ListBoxRow) -> std::cmp::Ordering {
        use std::cmp::Ordering::Equal;
        let rows = self.rows.borrow();
        let (Some((ia, fa)), Some((ib, fb))) = (rows.get(a), rows.get(b)) else { return Equal };
        let stats = self.stats.borrow();
        let most = |of: fn(&Stats) -> u32| stats.get(&fb.url).map(of).cmp(&stats.get(&fa.url).map(of));
        let by = match SORTS[self.sort.get()].0 {
            "views" => most(|s| s.views),
            "likes" => most(|s| s.likes),
            "newest" => fb.id.cmp(&fa.id),
            "az" => fa.title.to_lowercase().cmp(&fb.title.to_lowercase()),
            _ => Equal,
        };
        by.then(ia.cmp(ib))
    }

    fn row(self: &Rc<Self>, f: &Found, stats: &gtk::Box) -> gtk::ListBoxRow {
        let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
        play.add_css_class("flat");
        play.add_css_class("circular");
        play.set_valign(gtk::Align::Center);
        play.set_tooltip_text(Some(t("Listen")));

        let title = label(&f.title, &["vn-find-title"]);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_tooltip_text(Some(&f.title));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&title);
        text.append(stats);

        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("suggested-action");
        add.add_css_class("circular");
        add.set_valign(gtk::Align::Center);
        add.set_tooltip_text(Some(t("Add to soundboard")));

        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        b.add_css_class("vn-find-row");
        b.append(&play);
        b.append(&text);
        b.append(&add);
        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_child(Some(&b));

        let f = Rc::new(f.clone());
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

/// The likes and views of a sound under its title: a heart and an eye with their counts. Hidden
/// until they are known.
fn stats_line() -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    line.add_css_class("vn-find-stats");
    for (class, icon) in [("vn-find-likes", "heart"), ("vn-find-views", "eye")] {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        b.add_css_class(class);
        b.append(&line_icon(icon, 13));
        b.append(&gtk::Label::new(None));
        line.append(&b);
    }
    line.set_visible(false);
    line
}

fn show_stats(line: &gtk::Box, s: Stats) {
    let parts = [
        (s.likes, t("{} people like this on MyInstants")),
        (s.views, t("{} views on MyInstants")),
    ];
    let mut child = line.first_child();
    for (n, tip) in parts {
        let Some(b) = child else { break };
        if let Some(l) = b.last_child().and_downcast::<gtk::Label>() {
            l.set_label(&short_count(n));
        }
        b.set_tooltip_text(Some(&fill(tip, &[&n.to_string()])));
        child = b.next_sibling();
    }
    line.set_visible(true);
}

/// A count in few letters: 950, 3.2K, 12K, 1.2M. German writes a comma.
fn short_count(n: u32) -> String {
    let text = short_count_en(n);
    if crate::i18n::language() == "de" { text.replace('.', ",") } else { text }
}

fn short_count_en(n: u32) -> String {
    let short = |v: f64, unit: &str| {
        // Cut, never rounded up: 999,999 is 999K, not 1000K.
        let text = if v < 10.0 { format!("{v:.1}") } else { format!("{}", v.floor()) };
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => short((n / 100) as f64 / 10.0, "K"),
        _ => short((n / 100_000) as f64 / 10.0, "M"),
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

#[cfg(test)]
mod tests {
    #[test]
    fn counts_get_short() {
        let short = super::short_count_en;
        assert_eq!(short(0), "0");
        assert_eq!(short(950), "950");
        assert_eq!(short(1000), "1K");
        assert_eq!(short(3276), "3.2K");
        assert_eq!(short(12_345), "12K");
        assert_eq!(short(999_999), "999K");
        assert_eq!(short(1_250_000), "1.2M");
    }
}
