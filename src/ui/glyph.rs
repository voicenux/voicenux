//! Lucide icons (ISC) as paintables, for the sidebar and the settings. Each is SVG path data on
//! a 24 × 24 grid in `data/lucide/icons.tsv`, made by `scripts/fetch-lucide.py`, drawn with 2 px
//! lines. So they stay sharp at every size and take the CSS `color` of the image that shows them.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

/// Icons for names that are no icon.
const FALLBACK: &str = "sparkles";

/// The path data of an icon: lines, and the few filled dots.
fn find(name: &str) -> Option<(&'static str, &'static str)> {
    include_str!("../../data/lucide/icons.tsv").lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let mut cols = l.split('\t');
        (cols.next()? == name).then(|| (cols.next().unwrap_or(""), cols.next().unwrap_or("")))
    })
}

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Default)]
    pub struct Glyph {
        pub stroke: OnceCell<gsk::Path>,
        pub fill: OnceCell<Option<gsk::Path>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Glyph {
        const NAME: &'static str = "VnGlyph";
        type Type = super::Glyph;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for Glyph {}

    impl PaintableImpl for Glyph {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE | gdk::PaintableFlags::STATIC_CONTENTS
        }

        fn intrinsic_width(&self) -> i32 {
            24
        }

        fn intrinsic_height(&self) -> i32 {
            24
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            self.draw(snapshot, width, height, &gdk::RGBA::WHITE);
        }
    }

    impl SymbolicPaintableImpl for Glyph {
        fn snapshot_symbolic(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA]) {
            self.draw(snapshot, width, height, colors.first().unwrap_or(&gdk::RGBA::WHITE));
        }
    }

    impl Glyph {
        fn draw(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, color: &gdk::RGBA) {
            let (Some(s), Some(stroke)) = (snapshot.downcast_ref::<gtk::Snapshot>(), self.stroke.get()) else { return };
            let size = width.min(height) as f32;
            s.save();
            s.translate(&graphene::Point::new((width as f32 - size) / 2.0, (height as f32 - size) / 2.0));
            s.scale(size / 24.0, size / 24.0);
            if let Some(Some(fill)) = self.fill.get() {
                s.append_fill(fill, gsk::FillRule::Winding, color);
            }
            let line = gsk::Stroke::new(2.0);
            line.set_line_cap(gsk::LineCap::Round);
            line.set_line_join(gsk::LineJoin::Round);
            s.append_stroke(stroke, &line, color);
            s.restore();
        }
    }
}

glib::wrapper! {
    pub struct Glyph(ObjectSubclass<imp::Glyph>) @implements gdk::Paintable, gtk::SymbolicPaintable;
}

thread_local! {
    static GLYPHS: RefCell<HashMap<String, Glyph>> = RefCell::new(HashMap::new());
}

/// The icon `name`, or sparkles when there is none of that name. Made once, then shared.
pub fn glyph(name: &str) -> Glyph {
    let name = if find(name).is_some() { name } else { FALLBACK };
    GLYPHS.with(|g| {
        g.borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| {
                let (stroke, fill) = find(name).expect("the fallback is an icon");
                let glyph: Glyph = glib::Object::new();
                let imp = glyph.imp();
                let _ = imp.stroke.set(gsk::Path::parse(stroke).expect("icons.tsv has good paths"));
                let _ = imp.fill.set((!fill.is_empty()).then(|| gsk::Path::parse(fill).expect("icons.tsv has good paths")));
                glyph
            })
            .clone()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn icons_of_the_app_are_there() {
        for (_, icon, _) in crate::ui::window::PAGES {
            assert!(super::find(icon).is_some(), "no icon {icon}");
        }
        assert!(super::find(super::FALLBACK).is_some());
        for l in include_str!("../../data/lucide/icons.tsv").lines().filter(|l| !l.starts_with('#')) {
            assert!(l.split('\t').nth(1).is_some_and(|p| p.starts_with('M')), "bad path: {l}");
        }
    }
}
