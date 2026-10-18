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
use crate::soundboard::{RATE, SoundData, decode_bytes, level};
use crate::state::{Change, St};

/// A downloaded sound: the file to keep and the audio to play.
type Audio = Rc<(Vec<u8>, Arc<SoundData>)>;
type Waiting = Vec<Box<dyn FnOnce(Result<Audio, online::Error>)>>;

/// MyInstants shows 36 sounds a page. A page with fewer than this was the last one: a few can go
/// missing, like videos.
const FULL_PAGE: usize = 20;

/// Sounds load ahead, so they play at once: the top of the list, what scrolls into sight and the
/// one under the pointer, six at a time. One download takes up to a second, so one after the
/// other was too slow.
const AHEAD: i32 = 10;
const PARALLEL: usize = 6;
/// The likes and views of each sound are on a page of their own: eight are read at a time, those
/// in sight first. Three at a time took 9 s for a list of 36.
const NUMBERS_PARALLEL: usize = 8;
/// How much audio stays in memory. A sound of a few seconds takes 1 to 2 MB, 30 s take 11 MB.
const KEEP_BYTES: usize = 100 << 20;

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
    /// Sounds downloaded lately, by file, the last one used at the back.
    audio: RefCell<VecDeque<(String, Audio)>>,
    /// Sounds on their way, with what waits for them.
    pending: RefCell<HashMap<String, Waiting>>,
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
    /// The search words of the list shown, "" for the popular sounds.
    shown: RefCell<String>,
    /// Where each page starts in the list shown. Each page stays a block of its own, sorted in
    /// itself: a sorted list that took in new sounds jumped around.
    blocks: RefCell<Vec<usize>>,
    /// Pages read for each search, and whether MyInstants has more.
    pages: RefCell<HashMap<String, (u32, bool)>>,
    /// The spinner under the list while the next page loads, and whether it loads.
    more: adw::Spinner,
    loading_more: Cell<bool>,
    /// Sounds to load ahead, the next one first, and how many load right now. A new list starts a
    /// new round: what still loads for the old one takes no place from it.
    ahead: RefCell<VecDeque<Rc<Found>>>,
    loading: Cell<usize>,
    round: Cell<u32>,
    /// Sounds whose numbers are still to read, the next one first, with the line that shows
    /// them, and how many are read right now. They go by rounds too.
    numbers: RefCell<VecDeque<(Rc<Found>, gtk::Box)>>,
    reading: Cell<usize>,
    in_sight_soon: Cell<bool>,
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
    // Under the list while the next page loads. It loads by itself before the end comes in sight.
    let more = adw::Spinner::new();
    more.set_size_request(24, 24);
    more.set_halign(gtk::Align::Center);
    more.set_margin_top(10);
    more.set_margin_bottom(14);
    more.set_visible(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&list);
    content.append(&more);
    let scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&content).build();
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

    // How loud they play here. Added ones play at their own volume.
    let volume = super::soundboard::volume_slider(st.config.borrow().find_volume as f64, 1.0, {
        let st = st.clone();
        move |v| st.set_find_volume(v as f32)
    });
    volume.set_tooltip_text(Some(t("Volume of the sounds here")));
    volume.set_margin_top(8);
    volume.set_margin_start(16);
    volume.set_margin_end(16);

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
    panel.append(&volume);
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
        pending: RefCell::default(),
        stats: RefCell::default(),
        rows: RefCell::default(),
        sort: Cell::new(chosen),
        resort_soon: Cell::new(false),
        playing: RefCell::default(),
        listen_no: Cell::new(0),
        shown: RefCell::default(),
        pages: RefCell::default(),
        blocks: RefCell::default(),
        more,
        loading_more: Cell::new(false),
        ahead: RefCell::default(),
        loading: Cell::new(0),
        round: Cell::new(0),
        numbers: RefCell::default(),
        reading: Cell::new(0),
        in_sight_soon: Cell::new(false),
    });
    // What scrolls into sight loads ahead too.
    scroll.vadjustment().connect_value_changed({
        let finder = Rc::downgrade(&finder);
        move |adj| {
            let Some(f) = finder.upgrade() else { return };
            f.load_in_sight(adj);
            // One screen before the end, so the next sounds are there when it comes.
            if adj.value() + 2.0 * adj.page_size() >= adj.upper() {
                f.load_more();
            }
        }
    });
    finder.list.set_sort_func({
        let finder = Rc::downgrade(&finder);
        move |a, b| finder.upgrade().map_or(std::cmp::Ordering::Equal, |f| f.order(a, b)).into()
    });
    // Enter on a row listens, like its play button. Clicks are the row's own, see `row`.
    finder.list.connect_row_activated({
        let finder = Rc::downgrade(&finder);
        move |_, row| {
            let Some(me) = finder.upgrade() else { return };
            let found = me.rows.borrow().get(row).map(|(_, f)| f.clone());
            let play = row.child().and_then(|b| b.first_child()).and_downcast::<gtk::Button>();
            if let (Some(f), Some(play)) = (found, play) {
                me.listen(&f, &play);
            }
        }
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
            move || if q.is_empty() { online::popular(1) } else { online::search(&q, 1) },
            move |r| {
                if me.search_no.get() != no {
                    return;
                }
                match r {
                    Ok(found) => {
                        me.pages.borrow_mut().insert(query.clone(), (1, found.len() >= FULL_PAGE));
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
        self.shown.replace(query.to_string());
        self.more.set_visible(false);
        self.blocks.replace(vec![0]);
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
        let waiting = self.append(found, 0);
        self.list.invalidate_sort();
        self.stack.set_visible_child_name("results");
        // The sounds at the top load ahead, the first one first. Those of an older search wait no
        // longer.
        self.ahead.borrow_mut().clear();
        self.round.set(self.round.get() + 1);
        self.loading.set(0);
        for i in (0..AHEAD).rev() {
            let found = self.list.row_at_index(i).and_then(|r| self.rows.borrow().get(&r).map(|(_, f)| f.clone()));
            if let Some(f) = found {
                self.load_ahead(f);
            }
        }
        // The numbers in the order of the list as it is sorted.
        let mut waiting: Vec<_> = waiting.into_iter().collect();
        waiting.sort_by_key(|(_, line)| row_index(line));
        self.numbers.replace(waiting.into());
        self.reading.set(0);
        self.next_stats();
    }

    /// Puts rows for `found` in the list, the first one at place `start` in the order MyInstants
    /// gave. Returns the sounds whose numbers are not read yet.
    fn append(self: &Rc<Self>, found: Vec<Found>, start: usize) -> VecDeque<(Rc<Found>, gtk::Box)> {
        let mut waiting = VecDeque::new();
        for (i, f) in found.into_iter().enumerate() {
            let line = stats_line();
            let row = self.row(&f, &line);
            self.list.append(&row);
            let f = Rc::new(f);
            self.rows.borrow_mut().insert(row, (start + i, f.clone()));
            match self.stats.borrow().get(&f.url) {
                Some(s) => show_stats(&line, s),
                None => waiting.push_back((f, line)),
            }
        }
        waiting
    }

    /// Reads the next page of the list shown and puts its sounds at the end.
    fn load_more(self: &Rc<Self>) {
        let query = self.shown.borrow().clone();
        let Some((page, true)) = self.pages.borrow().get(&query).copied() else { return };
        if self.loading_more.replace(true) {
            return;
        }
        self.more.set_visible(true);
        let (me, no, q) = (self.clone(), self.search_no.get(), query.clone());
        off_thread(
            move || if q.is_empty() { online::popular(page + 1) } else { online::search(&q, page + 1) },
            move |r| {
                me.loading_more.set(false);
                me.more.set_visible(false);
                if me.search_no.get() != no {
                    return;
                }
                let found = match r {
                    Ok(found) => found,
                    Err(e) => {
                        me.st.notify(Change::Notice(error_text(e).to_string()));
                        return;
                    }
                };
                let full = found.len() >= FULL_PAGE;
                let mut results = me.results.borrow_mut();
                let all = results.entry(query.clone()).or_default();
                let new: Vec<Found> = found.into_iter().filter(|f| !all.iter().any(|a| a.url == f.url)).collect();
                let start = all.len();
                all.extend(new.iter().cloned());
                let total = all.len();
                drop(results);
                // A short page, or one with nothing new: that was all.
                let more = full && !new.is_empty();
                me.pages.borrow_mut().insert(query.clone(), (page + 1, more));
                me.blocks.borrow_mut().push(start);
                if !query.is_empty() {
                    me.heading.set_label(&fill(t("{} sounds"), &[&total.to_string()]));
                }
                let waiting = me.append(new, start);
                me.list.invalidate_sort();
                me.numbers.borrow_mut().extend(waiting);
                me.next_stats();
            },
        );
    }

    /// Reads the numbers of the next sounds in line, as long as fewer than [`NUMBERS_PARALLEL`]
    /// are read.
    fn next_stats(self: &Rc<Self>) {
        while self.reading.get() < NUMBERS_PARALLEL {
            let Some((found, line)) = self.numbers.borrow_mut().pop_front() else { return };
            let known = self.stats.borrow().get(&found.url).cloned();
            if let Some(s) = known {
                show_stats(&line, &s);
                continue;
            }
            self.reading.set(self.reading.get() + 1);
            let (me, round, sent) = (self.clone(), self.round.get(), (*found).clone());
            off_thread(
                move || online::stats(&sent),
                move |r| {
                    // Kept for an older list too: they show when it comes back.
                    if let Ok(s) = &r {
                        me.stats.borrow_mut().insert(found.url.clone(), s.clone());
                    }
                    if me.round.get() != round {
                        return;
                    }
                    me.reading.set(me.reading.get() - 1);
                    match r {
                        Ok(s) => {
                            show_stats(&line, &s);
                            me.resort();
                        }
                        // MyInstants wants a break: the rest stays without numbers.
                        Err(online::Error::TooMany) => me.numbers.borrow_mut().clear(),
                        Err(_) => {}
                    }
                    me.next_stats();
                },
            );
        }
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

    /// The order of two rows: by their block first, see `blocks`. In it, sounds without numbers
    /// yet go after those with, and the order MyInstants gave settles the rest.
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
        let blocks = self.blocks.borrow();
        let block = |i: &usize| blocks.iter().rev().find(|start| *start <= i).copied().unwrap_or(0);
        block(ia).cmp(&block(ib)).then(by).then(ia.cmp(ib))
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

        let add = gtk::Button::from_icon_name(download_icon());
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
        row.set_child(Some(&b));
        row.set_cursor_from_name(Some("pointer"));

        let f = Rc::new(f.clone());
        play.connect_clicked({
            let (me, f) = (self.clone(), f.clone());
            move |b| me.listen(&f, b)
        });
        // A click anywhere on the row listens or stops, like its play button, each of quick
        // clicks too: the list would take the second one as a double click and do nothing.
        // Clicks on the buttons stay theirs: a button takes a click only when it ends, so taking
        // every press here stole them, and the add button only played the sound.
        let click = gtk::GestureClick::new();
        click.connect_pressed(|g, _, x, y| {
            let state =
                if on_button(g, x, y) { gtk::EventSequenceState::Denied } else { gtk::EventSequenceState::Claimed };
            g.set_state(state);
        });
        click.connect_released({
            let (me, f, play) = (self.clone(), f.clone(), play.clone());
            move |g, _, x, y| {
                if g.widget().is_some_and(|w| w.contains(x, y)) && !on_button(g, x, y) {
                    me.listen(&f, &play);
                }
            }
        });
        row.add_controller(click);
        // The pointer on a row comes before a click on it: the sound loads already, without waiting
        // in line.
        let hover = gtk::EventControllerMotion::new();
        hover.connect_enter({
            let (me, f) = (self.clone(), f.clone());
            move |_, _, _| me.fetch(&f, |_| {})
        });
        row.add_controller(hover);
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
            me.st.preview(audio.1.clone(), me.st.config.borrow().find_volume);
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
        if button.has_css_class("vn-added") || button.child().is_some_and(|c| c.is::<adw::Spinner>()) {
            return;
        }
        let me = self.clone();
        let f2 = f.clone();
        self.with_audio(f, button, download_icon(), move |button, audio| {
            let add = {
                let (me, f, button) = (me.clone(), f2.clone(), button.clone());
                move |stats: Option<&Stats>| {
                    let icon = picture(&f.title, stats).unwrap_or("loudspeaker");
                    match me.st.add_found(&f.title, &f.ext, &audio.0, icon) {
                        Ok(()) => {
                            button.set_icon_name("object-select-symbolic");
                            button.add_css_class("vn-added");
                            button.set_tooltip_text(Some(t("Added")));
                            me.st.notify(Change::Notice(fill(t("{} is on your soundboard now"), &[&f.title])));
                        }
                        Err(e) => {
                            log::warn!("could not add {}: {e}", f.title);
                            me.st.notify(Change::Notice(fill(t("Could not add {}"), &[&f.title])));
                        }
                    }
                }
            };
            // A name that says nothing, like "Sven!": its page may tell more. Read it, unless
            // that happened already for its numbers.
            let known = me.stats.borrow().get(&f2.url).cloned();
            if known.is_some() || super::emoji::guess(&f2.title).is_some() {
                add(known.as_ref());
                return;
            }
            button.set_child(Some(&adw::Spinner::new()));
            let (sent, button) = ((*f2).clone(), button.clone());
            off_thread(
                move || online::stats(&sent),
                move |r| {
                    button.set_icon_name(download_icon());
                    add(r.as_ref().ok());
                },
            );
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
        if let Some(audio) = self.cached(&f.url) {
            then(button, audio);
            return;
        }
        if button.child().is_some_and(|c| c.is::<adw::Spinner>()) {
            return;
        }
        button.set_child(Some(&adw::Spinner::new()));
        let (me, button) = (self.clone(), button.clone());
        self.fetch(f, move |r| {
            button.set_icon_name(icon);
            match r {
                Ok(audio) => then(&button, audio),
                Err(e) => me.st.notify(Change::Notice(error_text(e).to_string())),
            }
        });
    }

    /// The audio of a sound, from memory or downloaded, for `then`. A sound on its way already
    /// is not asked for again: `then` waits for it.
    fn fetch(self: &Rc<Self>, f: &Found, then: impl FnOnce(Result<Audio, online::Error>) + 'static) {
        if let Some(audio) = self.cached(&f.url) {
            then(Ok(audio));
            return;
        }
        if let Some(waiting) = self.pending.borrow_mut().get_mut(&f.url) {
            waiting.push(Box::new(then));
            return;
        }
        self.pending.borrow_mut().insert(f.url.clone(), vec![Box::new(then)]);
        let (me, found, url) = (self.clone(), f.clone(), f.url.clone());
        off_thread(
            move || {
                let bytes = online::download(&found)?;
                // Most sounds there are very loud: they play as loud as the starter sounds.
                let data = level(decode_bytes(bytes.clone(), &found.ext).map_err(|_| online::Error::Failed)?);
                Ok::<_, online::Error>((bytes, Arc::new(data)))
            },
            move |r| {
                let r = r.map(|(bytes, data)| {
                    let audio: Audio = Rc::new((bytes, data));
                    me.keep(&url, audio.clone());
                    audio
                });
                let waiting = me.pending.borrow_mut().remove(&url).unwrap_or_default();
                for then in waiting {
                    then(r.clone());
                }
            },
        );
    }

    /// Loads a sound ahead, before it is played: it goes first in line.
    fn load_ahead(self: &Rc<Self>, f: Rc<Found>) {
        if self.has(&f.url) || self.pending.borrow().contains_key(&f.url) {
            return;
        }
        let mut ahead = self.ahead.borrow_mut();
        ahead.retain(|x| x.url != f.url);
        ahead.push_front(f);
        // What scrolled out of sight long ago waits no longer.
        ahead.truncate(30);
        drop(ahead);
        self.next_ahead();
    }

    /// Starts loading the next sounds in line, as long as fewer than [`PARALLEL`] load.
    fn next_ahead(self: &Rc<Self>) {
        while self.loading.get() < PARALLEL {
            let Some(f) = self.ahead.borrow_mut().pop_front() else { return };
            if self.has(&f.url) || self.pending.borrow().contains_key(&f.url) {
                continue;
            }
            self.loading.set(self.loading.get() + 1);
            let (me, round) = (self.clone(), self.round.get());
            self.fetch(&f, move |r| {
                if me.round.get() != round {
                    return;
                }
                me.loading.set(me.loading.get() - 1);
                // MyInstants wants a break: the rest loads when you listen.
                if matches!(r, Err(online::Error::TooMany)) {
                    me.ahead.borrow_mut().clear();
                }
                me.next_ahead();
            });
        }
    }

    /// Loads the rows in sight ahead, and a few below them, a moment after scrolling.
    fn load_in_sight(self: &Rc<Self>, adj: &gtk::Adjustment) {
        if self.in_sight_soon.replace(true) {
            return;
        }
        let (me, adj) = (self.clone(), adj.clone());
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            me.in_sight_soon.set(false);
            let first = me.list.row_at_y(adj.value() as i32).map_or(0, |r| r.index());
            let last = me.list.row_at_y((adj.value() + adj.page_size()) as i32).map_or(first + 12, |r| r.index()) + 3;
            // Bottom up, so the top one goes first in line.
            for i in (first..=last).rev() {
                let found = me.list.row_at_index(i).and_then(|r| me.rows.borrow().get(&r).map(|(_, f)| f.clone()));
                if let Some(f) = found {
                    me.load_ahead(f);
                }
            }
            // Their numbers go first too, top to bottom. The others stay in their order.
            let mut numbers: Vec<_> = me.numbers.borrow_mut().drain(..).collect();
            numbers.sort_by_key(|(_, line)| {
                let i = row_index(line);
                if (first..=last).contains(&i) { (0, i) } else { (1, 0) }
            });
            me.numbers.borrow_mut().extend(numbers);
            me.next_stats();
        });
    }

    /// Whether a sound is in memory already.
    fn has(&self, url: &str) -> bool {
        self.audio.borrow().iter().any(|(u, _)| u == url)
    }

    fn cached(&self, url: &str) -> Option<Audio> {
        let mut audio = self.audio.borrow_mut();
        let at = audio.iter().position(|(u, _)| u == url)?;
        let item = audio.remove(at)?;
        let found = item.1.clone();
        audio.push_back(item);
        Some(found)
    }

    /// Keeps a sound in memory. The ones not used for longest go when they take more than
    /// [`KEEP_BYTES`] together.
    fn keep(&self, url: &str, a: Audio) {
        let mut audio = self.audio.borrow_mut();
        audio.retain(|(u, _)| u != url);
        audio.push_back((url.to_string(), a));
        let size = |a: &Audio| a.0.len() + a.1.frames.len() * std::mem::size_of::<[f32; 2]>();
        let mut total: usize = audio.iter().map(|(_, a)| size(a)).sum();
        while total > KEEP_BYTES && audio.len() > 1 {
            if let Some((_, gone)) = audio.pop_front() {
                total -= size(&gone);
            }
        }
    }
}

/// The likes and views of a sound under its title: a heart and an eye with their counts. Not to
/// be seen until they are known.
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
    // Its place is kept while the numbers load, so the rows below do not move down when they come.
    line.add_css_class("unknown");
    line
}

fn show_stats(line: &gtk::Box, s: &Stats) {
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
    line.remove_css_class("unknown");
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

/// Whether a click at `x`, `y` in the widget of `g` is on one of the buttons in it.
fn on_button(g: &gtk::GestureClick, x: f64, y: f64) -> bool {
    let Some(w) = g.widget() else { return false };
    let hit = w.pick(x, y, gtk::PickFlags::DEFAULT);
    std::iter::successors(hit, |h| h.parent()).take_while(|h| *h != w).any(|h| h.is::<gtk::Button>())
}

/// The place in the list of the row a line of numbers is in.
fn row_index(line: &gtk::Box) -> i32 {
    line.ancestor(gtk::ListBoxRow::static_type()).and_downcast::<gtk::ListBoxRow>().map_or(i32::MAX, |r| r.index())
}

/// A picture for a found sound: one its name fits, or else one what its page says fits, or else
/// one for its category, like a clown for a prank.
fn picture(title: &str, stats: Option<&Stats>) -> Option<&'static str> {
    super::emoji::guess(title).or_else(|| {
        let stats = stats?;
        super::emoji::guess(&stats.about).or(match stats.category.as_str() {
            "anime & manga" => Some("fish-cake-with-swirl"),
            "games" => Some("video-game"),
            "memes" => Some("joy"),
            "movies" => Some("clapper-board"),
            "music" => Some("musical-note"),
            "politics" => Some("classical-building"),
            "pranks" => Some("clown"),
            "reactions" => Some("astonished-face"),
            "sports" => Some("soccer-ball"),
            "television" => Some("television"),
            "tiktok trends" => Some("mobile-phone"),
            "viral" => Some("fire"),
            "whatsapp audios" => Some("speech-balloon"),
            _ => None,
        })
    })
}

/// The icon to add a found sound: a download. Breeze has a plain one, Adwaita only one with a
/// folder.
fn download_icon() -> &'static str {
    let plain =
        gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon("download-symbolic"));
    if plain { "download-symbolic" } else { "folder-download-symbolic" }
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
