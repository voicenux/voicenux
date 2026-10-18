//! The picture picker: all Fluent emoji in a grid, with a search in English and German.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;

use super::{fold, icon, label, load_picture, popup, smooth_scroll};
use crate::i18n::{language, t};

/// One picture of `data/emoji.tsv`.
struct Emoji {
    id: &'static str,
    /// In the language of the user.
    name: &'static str,
    /// Names and keywords in both languages, folded for search.
    words: String,
}

fn index() -> &'static [Emoji] {
    static INDEX: OnceLock<Vec<Emoji>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let german = language() == "de";
        include_str!("../../data/emoji.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut cols = l.split('\t');
                let (id, en, de, words) = (cols.next()?, cols.next()?, cols.next()?, cols.next()?);
                Some(Emoji { id, name: if german { de } else { en }, words: fold(words) })
            })
            .collect()
    })
}


fn find(id: &str) -> Option<&'static Emoji> {
    static BY_ID: OnceLock<HashMap<&'static str, &'static Emoji>> = OnceLock::new();
    BY_ID.get_or_init(|| index().iter().map(|e| (e.id, e)).collect()).get(id).copied()
}

/// Words of sound names that say nothing about the sound, in English and German. Some would even
/// pick a wrong picture: "die" is a die, "bin" a trash can, "hell" (bright) a light bulb.
const SKIP: &str = "the and with for you your are was not but all out get got getting let lets this that what \
    from have has had his her its who why how when off one now just like very really yes man bro yeah der die das \
    den dem des und ein eine einer einen ist bin bist sind mit auf aus fur von zum zur nicht ich sie wir ihr mich \
    dich mir dir mach muss hab habe jetzt halt nein doch noch mal wie wer oder aber dann denn schon sehr kann hell \
    sch sound sounds effect effects sfx meme memes loud louder bass boosted boost version remix edit original full \
    short long funny best new old ultra quality low high button";

/// Words of sound names that the emoji names lack, and their picture. These go before the names of
/// the pictures: "Fart ringtone" is a fart.
const STRONG: &[(&str, &str)] = &[
    ("pile-of-poo", "fart farts furz furze pups pupsen poop poo shit scheisse scheissen kacke kacken dunnpfiff"),
    ("alarm-clock", "aufstehen steh wake wecker"),
    ("police-car-light", "siren sirene"),
    ("zzz", "snore schnarchen"),
    ("cross-mark", "buzzer wrong falsch"),
    ("check-mark-button", "correct richtig"),
    ("warning", "error fehler"),
    ("party-popper", "yay yippee yippe yippie hurra juhu"),
    ("unamused-face", "bruh"),
    ("face-with-raised-eyebrow", "huh"),
    ("astonished-face", "wow omg god"),
    ("cat", "meow miau"),
    ("t-rex", "rawr dinosaur dino"),
    ("water-pistol", "gun gunshot shotgun schuss"),
    ("oncoming-fist", "punch"),
    ("face-screaming-in-fear", "scream schrei schreien fah aaa"),
    ("trophy", "win victory sieg gewonnen"),
    ("thumbs-down", "fail lose verloren womp"),
    ("door", "knock klopfen"),
    ("sponge", "spongebob"),
    ("doughnut", "homer"),
    ("mushroom", "mario"),
    ("smiling-face-with-hearts", "aww"),
    ("balloon", "pop"),
    ("zipper-mouth-face", "shut maul"),
    ("droplet", "drip"),
    ("pill", "risiken nebenwirkungen"),
    ("person-running", "run"),
    ("broken-heart", "damage"),
];

/// Words that only say what kind of sound it is. They go after the names of the pictures:
/// "Elevator music" is an elevator, "German song" a song.
const WEAK: &[(&str, &str)] = &[
    ("musical-note", "song songs lied lieder music musik musiic melody melodie jingle intro outro"),
    ("mobile-phone", "ringtone klingelton handy whatsapp"),
    ("bell", "notification benachrichtigung ding beep piep"),
    ("video-game", "fortnite roblox minecraft gaming"),
];

/// A picture that fits a sound name, like the dog for "LBS FX Dog Small Alert Bark". First the
/// strong words of sounds, then a picture named just like a word ("dog", "dog face"), one with the
/// word in its name, one with it as a keyword, the weak words of sounds, and last a picture whose
/// keyword starts or ends a long word, for German ones like "Katzenvieh".
pub fn guess(name: &str) -> Option<&'static str> {
    let words = sound_words(name);
    let listed = |list: &[(&'static str, &str)]| {
        words.iter().find_map(|w| list.iter().find(|(_, ws)| ws.split(' ').any(|x| x == w)).map(|(id, _)| *id))
    };
    if let Some(id) = listed(STRONG) {
        return Some(id);
    }
    let named = |e: &Emoji, w: &str| e.id == w || e.id.strip_suffix("-face") == Some(w);
    let in_name = |e: &Emoji, w: &str| e.id.split('-').any(|x| x == w);
    let keyword = |e: &Emoji, w: &str| e.words.split(' ').any(|x| x == w);
    for test in [&named as &dyn Fn(&Emoji, &str) -> bool, &in_name, &keyword] {
        if let Some(e) = words.iter().find_map(|w| index().iter().find(|e| test(e, w))) {
            return Some(e.id);
        }
    }
    if let Some(id) = listed(WEAK) {
        return Some(id);
    }
    let part = |e: &Emoji, w: &str| {
        w.len() > 5
            && e.words.split(' ').any(|k| k.len() >= 5 && w.len() > k.len() && (w.starts_with(k) || w.ends_with(k)))
    };
    words.iter().find_map(|w| index().iter().find(|e| part(e, w))).map(|e| e.id)
}

/// The words of a sound name to look for, the last first: in English they are mostly the thing
/// itself. A stretched word also counts short ("YAAAY" as "yay"), a plural also without its "s",
/// and a long cry like "AAAAH" as a scream.
fn sound_words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in fold(name).split(|c: char| !c.is_alphanumeric()).rev() {
        let mut v = vec![w.to_string()];
        let short = squeeze(w);
        if short.chars().count() <= 2 && w.chars().count() >= 3 && short.chars().all(|c| "aeiouh".contains(c)) {
            v.push("aaa".into());
        } else if short != w {
            v.push(short);
        }
        for x in v.clone() {
            if x.chars().count() >= 5
                && let Some(one) = x.strip_suffix('s')
            {
                v.push(one.to_string());
            }
        }
        for x in v {
            if x.chars().count() >= 3 && !SKIP.split_whitespace().any(|s| s == x) && !out.contains(&x) {
                out.push(x);
            }
        }
    }
    out
}

/// Runs of three or more of the same letter as one: "yaaaay" is "yay".
fn squeeze(w: &str) -> String {
    let chars: Vec<char> = w.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let run = chars[i..].iter().take_while(|c| **c == chars[i]).count();
        for _ in 0..if run >= 3 { 1 } else { run } {
            out.push(chars[i]);
        }
        i += run;
    }
    out
}

/// A button showing a picture. Clicking it opens a grid of all pictures to choose from.
pub fn icon_picker(current: &str, size: i32, on_pick: impl Fn(&str) + 'static) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_child(Some(&icon(current, size)));
    button.set_tooltip_text(Some(t("Change picture")));
    let popover = gtk::Popover::new();
    popup::menu(&popover);
    button.set_popover(Some(&popover));
    // The grid is made on the first open: there are many pickers, and most never open.
    let on_pick: Rc<dyn Fn(&str)> = Rc::new(on_pick);
    button.set_create_popup_func(move |b| {
        if popover.child().is_none() {
            popover.set_child(Some(&grid(&popover, b, size, on_pick.clone())));
        }
    });
    button
}

fn grid(popover: &gtk::Popover, button: &gtk::MenuButton, size: i32, on_pick: Rc<dyn Fn(&str)>) -> gtk::Box {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(t("Search emojis")));
    search.set_margin_bottom(8);

    let ids: Vec<&str> = index().iter().map(|e| e.id).collect();
    let query: Rc<RefCell<Vec<String>>> = Rc::default();
    let filter = gtk::CustomFilter::new({
        let query = query.clone();
        move |item| {
            let Some(id) = item.downcast_ref::<gtk::StringObject>().map(|s| s.string()) else { return false };
            let q = query.borrow();
            q.is_empty() || find(&id).is_some_and(|e| q.iter().all(|w| e.words.contains(w.as_str())))
        }
    });
    let model = gtk::FilterListModel::new(Some(gtk::StringList::new(&ids)), Some(filter.clone()));

    // Only the pictures on screen are loaded, and not kept: all of them would take a lot of memory.
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let img = gtk::Image::new();
        img.set_pixel_size(40);
        item.downcast_ref::<gtk::ListItem>().expect("a list item").set_child(Some(&img));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(img), Some(id)) = (
            item.child().and_downcast::<gtk::Image>(),
            item.item().and_downcast::<gtk::StringObject>().map(|s| s.string()),
        ) else {
            return;
        };
        img.set_paintable(Some(&load_picture(&id)));
        img.set_tooltip_text(find(&id).map(|e| e.name));
    });
    let view = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model.clone()))), Some(factory));
    view.add_css_class("vn-emoji-grid");
    view.set_min_columns(6);
    view.set_max_columns(8);
    view.set_single_click_activate(true);

    let pick: Rc<dyn Fn(&str)> = Rc::new({
        let (popover, button) = (popover.downgrade(), button.downgrade());
        move |id: &str| {
            if let (Some(popover), Some(button)) = (popover.upgrade(), button.upgrade()) {
                button.set_child(Some(&icon(id, size)));
                popover.popdown();
            }
            on_pick(id);
        }
    });
    view.connect_activate({
        let pick = pick.clone();
        move |v, pos| {
            if let Some(id) = v.model().and_then(|m| m.item(pos)).and_downcast::<gtk::StringObject>() {
                pick(&id.string());
            }
        }
    });

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(360)
        .min_content_width(440)
        .child(&view)
        .build();
    smooth_scroll(&scroll);
    let none = label(t("No pictures found"), &["dim-label"]);
    let results = gtk::Stack::new();
    results.add_named(&scroll, Some("grid"));
    results.add_named(&none, Some("none"));

    search.connect_search_changed({
        let (model, results, scroll) = (model.clone(), results.clone(), scroll.clone());
        move |s| {
            *query.borrow_mut() = fold(&s.text()).split_whitespace().map(String::from).collect();
            filter.changed(gtk::FilterChange::Different);
            results.set_visible_child_name(if model.n_items() == 0 { "none" } else { "grid" });
            scroll.vadjustment().set_value(0.0);
        }
    });
    // Enter takes the first picture found.
    search.connect_activate(move |_| {
        if let Some(id) = model.item(0).and_downcast::<gtk::StringObject>() {
            pick(&id.string());
        }
    });
    popover.connect_map({
        let search = search.clone();
        move |_| {
            search.grab_focus();
        }
    });
    popover.connect_closed({
        let search = search.clone();
        move |_| search.set_text("")
    });

    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_margin_top(10);
    b.set_margin_bottom(10);
    b.set_margin_start(10);
    b.set_margin_end(10);
    b.append(&search);
    b.append(&results);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(q: &str) -> Vec<&'static str> {
        let q: Vec<String> = fold(q).split_whitespace().map(String::from).collect();
        index().iter().filter(|e| q.iter().all(|w| e.words.contains(w.as_str()))).map(|e| e.id).collect()
    }

    #[test]
    fn every_picture_is_there() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(index().len() > 1500);
        for e in index() {
            let found = ["icons", "emoji"].iter().any(|d| std::path::Path::new(&format!("{root}/data/{d}/{}.png", e.id)).exists());
            assert!(found, "no picture for {}", e.id);
        }
    }

    #[test]
    fn search_finds_english_and_german() {
        assert!(search("mouse").contains(&"mouse"));
        assert!(search("Maus").contains(&"mouse-2"));
        assert!(search("mause").contains(&"mouse"));
        assert!(search("posthorn").contains(&"postal-horn"));
        assert!(search("grinning face").contains(&"grinning-face"));
        assert!(search("xyzzy").is_empty());
    }

    #[test]
    fn guesses_a_picture_for_a_sound() {
        for name in ["Dog barking", "LBS FX DOG Small Alert Bark001"] {
            let dog = guess(name).expect("a dog");
            assert!(dog.split('-').any(|w| w == "dog"), "{name}: {dog}");
        }
        assert_eq!(guess("xyzzy 01"), None);
    }

    #[test]
    fn guesses_pictures_for_meme_sounds() {
        for (name, picture) in [
            ("Fart", Some("pile-of-poo")),
            ("Furz Klingelton", Some("pile-of-poo")),
            ("Peppa Wutz - Stück Scheiße", Some("pile-of-poo")),
            ("German song", Some("musical-note")),
            ("Elevator Music Background", Some("elevator")),
            ("Aufstehen!!!", Some("alarm-clock")),
            ("YAAAAAAAAY", Some("party-popper")),
            ("FAHHHHHHH", Some("face-screaming-in-fear")),
            ("The Saxophones Getting Louder", Some("saxophone")),
            ("Gutenmorgen Sonnenschein", Some("sun-with-face")),
            ("Apple Pay", Some("red-apple")),
            ("sch… katzenvieh", Some("grinning-cat")),
            // "die" is no die here.
            ("Ralf Schumacher - Mitten in die Fresse", None),
        ] {
            assert_eq!(guess(name), picture, "{name}");
        }
    }

    #[test]
    fn sound_words_have_pictures() {
        for (id, _) in STRONG.iter().chain(WEAK) {
            assert!(find(id).is_some(), "no picture {id}");
        }
    }
}
