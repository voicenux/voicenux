//! Finds more AI voices on Hugging Face and adds them with one click: the download dialog asks
//! how high the voice is, and the voice lands among your own.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{clear, label, line_icon, smooth_scroll};
use crate::ai::{self, hub};
use crate::i18n::{fill, t};
use crate::state::St;

/// The sizes of repos are read this many at a time, those at the top first.
const PARALLEL: usize = 6;

type Sizes = HashMap<String, u64>;

thread_local! {
    /// What was searched last and found, so the dialog opens again where you left it.
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static RESULTS: RefCell<HashMap<String, Vec<hub::Hit>>> = RefCell::default();
    /// The sizes of the files of each repo, by repo.
    static SIZES: RefCell<HashMap<String, Sizes>> = RefCell::default();
}

struct Row {
    hit: RefCell<hub::Hit>,
    row: gtk::ListBoxRow,
    /// "Someone · 55 MB" under the name.
    about: gtk::Label,
}

struct Finder {
    st: St,
    dialog: glib::WeakRef<adw::Dialog>,
    list: gtk::ListBox,
    stack: gtk::Stack,
    message: gtk::Label,
    last: RefCell<String>,
    /// Counts searches, so an answer that comes late is dropped.
    search_no: Cell<u32>,
    rows: RefCell<Vec<Rc<Row>>>,
    /// Repos whose sizes are still to read, and how many are read right now.
    queue: RefCell<VecDeque<(String, String)>>,
    reading: Cell<usize>,
}

/// Opens the dialog, with `words` searched when there are any.
pub fn open(st: &St, parent: &impl IsA<gtk::Widget>, words: &str) {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(t("Search Hugging Face")));
    search.set_search_delay(400);
    search.set_hexpand(true);
    search.set_margin_top(4);
    search.set_margin_start(16);
    search.set_margin_end(16);
    search.set_margin_bottom(8);

    // Without a graphics card for them, nothing here would play.
    let unusable = label(
        t("AI voices need an NVIDIA or AMD graphics card, or Run on the processor in Settings."),
        &["warning", "caption"],
    );
    unusable.set_wrap(true);
    unusable.set_margin_start(16);
    unusable.set_margin_end(16);
    unusable.set_margin_bottom(8);
    unusable.set_visible(!ai::usable());

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("vn-find-list");
    let scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
    smooth_scroll(&scroll);

    let start = adw::StatusPage::builder()
        .paintable(&super::load_picture("robot"))
        .title(t("Find more AI voices"))
        .description(t("Type the name of a character, like Mario or Shrek."))
        .build();
    start.add_css_class("compact");
    let message = label("", &["dim-label"]);
    message.set_wrap(true);
    message.set_justify(gtk::Justification::Center);
    message.set_margin_start(24);
    message.set_margin_end(24);
    let spinner = adw::Spinner::new();
    spinner.set_size_request(32, 32);
    let loading = gtk::Box::new(gtk::Orientation::Vertical, 12);
    loading.set_valign(gtk::Align::Center);
    loading.append(&spinner);
    loading.append(&label(t("Searching Hugging Face…"), &["dim-label"]));

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vexpand(true);
    stack.add_named(&start, Some("start"));
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&scroll, Some("results"));
    stack.add_named(&message, Some("message"));

    let credit = label(
        t("Voices people made and shared on Hugging Face. Some sound great, some do not."),
        &["dim-label", "caption"],
    );
    credit.set_wrap(true);
    credit.set_justify(gtk::Justification::Center);
    credit.set_margin_top(8);
    credit.set_margin_bottom(12);
    credit.set_margin_start(16);
    credit.set_margin_end(16);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&search);
    body.append(&unusable);
    body.append(&stack);
    body.append(&credit);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    let dialog =
        adw::Dialog::builder().title(t("Find AI voices")).content_width(520).content_height(640).child(&view).build();

    let finder = Rc::new(Finder {
        st: st.clone(),
        dialog: dialog.downgrade(),
        list,
        stack,
        message,
        // Not a search anyone types, so the first one always runs.
        last: RefCell::new("\u{0}".into()),
        search_no: Cell::new(0),
        rows: RefCell::default(),
        queue: RefCell::default(),
        reading: Cell::new(0),
    });
    finder.list.connect_row_activated({
        let finder = Rc::downgrade(&finder);
        move |_, row| {
            let Some(me) = finder.upgrade() else { return };
            let found = me.rows.borrow().iter().find(|r| &r.row == row).cloned();
            if let Some(r) = found {
                me.add(&r, None);
            }
        }
    });
    search.connect_search_changed({
        let finder = finder.clone();
        move |s| finder.search(&s.text())
    });
    search.connect_activate({
        let finder = finder.clone();
        move |s| finder.search(&s.text())
    });
    // Escape empties the search first, a second one closes the dialog.
    search.connect_stop_search({
        let dialog = dialog.downgrade();
        move |s| {
            if !s.text().is_empty() {
                s.set_text("");
            } else if let Some(d) = dialog.upgrade() {
                d.close();
            }
        }
    });
    let words = if words.is_empty() { LAST.with_borrow(|l| l.clone()) } else { words.to_string() };
    search.set_text(&words);
    search.set_position(-1);
    finder.search(&words);
    dialog.set_focus(Some(&search));
    dialog.present(Some(parent));
}

impl Finder {
    fn search(self: &Rc<Self>, text: &str) {
        let words = text.trim().to_string();
        if *self.last.borrow() == words {
            return;
        }
        self.last.replace(words.clone());
        LAST.with_borrow_mut(|l| *l = words.clone());
        let no = self.search_no.get() + 1;
        self.search_no.set(no);
        if words.chars().count() < 2 {
            self.stack.set_visible_child_name("start");
            return;
        }
        if let Some(hits) = RESULTS.with_borrow(|r| r.get(&words).cloned()) {
            self.show(hits);
            return;
        }
        self.stack.set_visible_child_name("loading");
        let me = self.clone();
        let w = words.clone();
        off_thread(
            move || hub::search(&w),
            move |r| {
                if me.search_no.get() != no {
                    return;
                }
                match r {
                    Ok(hits) => {
                        RESULTS.with_borrow_mut(|r| r.insert(words, hits.clone()));
                        me.show(hits);
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

    fn show(self: &Rc<Self>, hits: Vec<hub::Hit>) {
        clear(&self.list);
        self.rows.borrow_mut().clear();
        self.queue.borrow_mut().clear();
        if hits.is_empty() {
            self.say(t("No AI voices found. Try other words."));
            return;
        }
        let mut repos: Vec<(String, String)> = Vec::new();
        for hit in hits {
            let key = (hit.source.repo.clone(), hit.source.commit.clone());
            if !repos.contains(&key) {
                repos.push(key);
            }
            let r = self.row(hit);
            self.list.append(&r.row);
            self.rows.borrow_mut().push(r.clone());
            self.sized(&r);
        }
        self.stack.set_visible_child_name("results");
        self.queue.replace(repos.into());
        self.next_sizes();
    }

    fn row(self: &Rc<Self>, hit: hub::Hit) -> Rc<Row> {
        let pic = super::icon(&picture(&self.st, &hit.source.name), 40);
        let title = label(&hit.source.name, &["vn-find-title"]);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_tooltip_text(Some(&format!("{}\n{}/{}", hit.source.name, hit.source.repo, hit.source.file)));
        let about = label("", &["dim-label", "caption", "numeric"]);
        about.set_xalign(0.0);
        about.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let under = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        under.append(&about);
        if hit.likes > 0 {
            let likes = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            likes.add_css_class("vn-find-likes");
            likes.append(&line_icon("heart", 13));
            likes.append(&label(&hit.likes.to_string(), &["caption", "numeric"]));
            likes.set_tooltip_text(Some(&fill(t("{} people like this on Hugging Face"), &[&hit.likes.to_string()])));
            under.append(&likes);
        }
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&title);
        text.append(&under);

        let page = gtk::Button::builder().child(&line_icon("globe", 16)).css_classes(["flat", "circular"]).build();
        page.set_valign(gtk::Align::Center);
        page.set_tooltip_text(Some(t("Open on Hugging Face")));
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("suggested-action");
        add.add_css_class("circular");
        add.set_valign(gtk::Align::Center);
        add.set_tooltip_text(Some(t("Add this voice")));
        add.set_sensitive(ai::usable());

        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        b.add_css_class("vn-find-row");
        b.append(&pic);
        b.append(&text);
        b.append(&page);
        b.append(&add);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&b));
        row.set_activatable(ai::usable());
        let repo = hit.source.repo.clone();
        let r = Rc::new(Row { hit: RefCell::new(hit), row, about });
        page.connect_clicked(move |b| {
            let window = b.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new(&hub::page(&repo)).launch(window.as_ref(), None::<&gtk::gio::Cancellable>, |_| {});
        });
        add.connect_clicked({
            let (me, r) = (self.clone(), r.clone());
            move |b| me.add(&r, Some(b))
        });
        r
    }

    /// Shows the size of a hit once its repo's sizes are known. One too big to download goes.
    fn sized(&self, r: &Row) {
        let mut hit = r.hit.borrow_mut();
        let owner = hit.source.repo.split('/').next().unwrap_or_default().to_string();
        let sizes = SIZES.with_borrow(|s| s.get(&hit.source.repo).map(|sizes| hub::mb(&hit.source, sizes)));
        match sizes {
            None => r.about.set_label(&owner),
            Some(Some(mb)) if mb as u64 * 1_000_000 <= ai::install::MAX_FOUND => {
                hit.source.mb = mb;
                r.about.set_label(&format!("{owner}  ·  {mb}\u{a0}MB"));
            }
            // Gone since the search, or too big.
            Some(_) => r.row.set_visible(false),
        }
    }

    /// Reads the sizes of the next repos in line, as long as fewer than [`PARALLEL`] are read.
    fn next_sizes(self: &Rc<Self>) {
        while self.reading.get() < PARALLEL {
            let Some((repo, commit)) = self.queue.borrow_mut().pop_front() else { return };
            if SIZES.with_borrow(|s| s.contains_key(&repo)) {
                continue;
            }
            self.reading.set(self.reading.get() + 1);
            let me = self.clone();
            self.read_sizes(repo, commit, move |_| {
                me.reading.set(me.reading.get() - 1);
                me.next_sizes();
            });
        }
    }

    fn read_sizes(self: &Rc<Self>, repo: String, commit: String, done: impl FnOnce(bool) + 'static) {
        let me = self.clone();
        let r = repo.clone();
        off_thread(
            move || hub::sizes(&r, &commit),
            move |result| {
                let ok = result.is_ok();
                if let Ok(sizes) = result {
                    SIZES.with_borrow_mut(|s| s.insert(repo.clone(), sizes));
                    for row in me.rows.borrow().iter().filter(|row| row.hit.borrow().source.repo == repo) {
                        me.sized(row);
                    }
                }
                done(ok);
            },
        );
    }

    /// Asks to download a hit, as soon as its size is known.
    fn add(self: &Rc<Self>, r: &Rc<Row>, button: Option<&gtk::Button>) {
        if !ai::usable() {
            return;
        }
        if r.hit.borrow().source.mb == 0 {
            let (repo, commit) = {
                let s = &r.hit.borrow().source;
                (s.repo.clone(), s.commit.clone())
            };
            if let Some(b) = button {
                b.set_sensitive(false);
            }
            let (me, r, b) = (self.clone(), r.clone(), button.cloned());
            self.read_sizes(repo, commit, move |ok| {
                if let Some(b) = &b {
                    b.set_sensitive(true);
                }
                if r.hit.borrow().source.mb > 0 && r.row.is_visible() {
                    me.add(&r, None);
                } else if !ok {
                    me.st.notify(crate::state::Change::Notice(t("That did not work. Try again.").into()));
                }
            });
            return;
        }
        let Some(dialog) = self.dialog.upgrade() else { return };
        let source = r.hit.borrow().source.clone();
        let v = self.st.found_voice(&source, &picture(&self.st, &source.name));
        super::ai::offer_found(&self.st, &dialog, v);
    }
}

/// A picture for a found voice: the one of a built-in AI voice of the same character, one its
/// name fits, or a robot.
fn picture(st: &St, name: &str) -> String {
    let plain = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let me = plain(name);
    let voices = st.voices.borrow();
    let builtin = voices.iter().find(|v| v.builtin && v.category == "ai" && me.contains(&plain(&v.name)));
    builtin.map_or_else(|| super::emoji::guess(name).unwrap_or("robot").to_string(), |v| v.icon.clone())
}

fn error_text(e: hub::Error) -> &'static str {
    match e {
        hub::Error::Offline => t("Could not reach Hugging Face. Are you online?"),
        hub::Error::TooMany => t("Too many searches. Wait a minute and try again."),
    }
}

/// Runs `work` on its own thread and hands the result to `done` on the main thread.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, hub::Error> + Send + 'static,
    done: impl FnOnce(Result<T, hub::Error>) + 'static,
) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        done(rx.recv().await.unwrap_or(Err(hub::Error::Offline)));
    });
}
