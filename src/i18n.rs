//! Translations. Texts are written in English in the code and wrapped in `t()`;
//! `po/<lang>.po` holds the other languages.

use std::collections::HashMap;
use std::sync::OnceLock;

const CATALOGS: &[(&str, &str)] = &[("de", include_str!("../po/de.po"))];

fn catalog() -> &'static HashMap<&'static str, &'static str> {
    static CAT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    CAT.get_or_init(|| {
        let lang = language();
        CATALOGS.iter().find(|(code, _)| *code == lang).map(|(_, text)| parse(text)).unwrap_or_default()
    })
}

/// The first language of the user that we have, or "en".
pub fn language() -> String {
    for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            for part in v.split(':') {
                let code = part.split(['_', '.', '@']).next().unwrap_or("");
                if code.is_empty() || code == "C" || code == "POSIX" {
                    continue;
                }
                if code == "en" || CATALOGS.iter().any(|(c, _)| *c == code) {
                    return code.to_string();
                }
            }
            if !v.is_empty() {
                break;
            }
        }
    }
    "en".into()
}

/// Translates `s`. Falls back to the English text.
pub fn t(s: &'static str) -> &'static str {
    catalog().get(s).copied().unwrap_or(s)
}

/// For texts that are not literals in the code, like voice names from the voice file.
pub fn t_dyn(s: &str) -> String {
    catalog().get(s).map(|v| v.to_string()).unwrap_or_else(|| s.to_string())
}

/// Replaces `{}` placeholders in order.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = template;
    for a in args {
        match rest.find("{}") {
            Some(i) => {
                out.push_str(&rest[..i]);
                out.push_str(a);
                rest = &rest[i + 2..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn unquote(line: &str) -> String {
    let inner = line.trim();
    let inner = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or("");
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A small PO reader: msgid/msgstr pairs with continued lines. Leaks the strings, they
/// live as long as the program anyway.
pub fn parse(text: &str) -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    let mut id = String::new();
    let mut val = String::new();
    let mut in_id = false;
    let mut flush = |id: &mut String, val: &mut String| {
        if !id.is_empty() && !val.is_empty() {
            map.insert(
                &*Box::leak(std::mem::take(id).into_boxed_str()),
                &*Box::leak(std::mem::take(val).into_boxed_str()),
            );
        }
        id.clear();
        val.clear();
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("msgid ") {
            flush(&mut id, &mut val);
            id = unquote(rest);
            in_id = true;
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            val = unquote(rest);
            in_id = false;
        } else if line.starts_with('"') {
            if in_id {
                id.push_str(&unquote(line));
            } else {
                val.push_str(&unquote(line));
            }
        }
    }
    flush(&mut id, &mut val);
    map
}

/// All literals passed to `t(...)` in the sources, for the completeness test.
#[cfg(test)]
pub fn source_strings() -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("i18n.rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                let mut rest = text.as_str();
                while let Some(i) = rest.find("t(\"") {
                    // Only the function itself, not names that end in "t(".
                    let before = rest[..i].chars().last();
                    rest = &rest[i + 3..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let mut s = String::new();
                    let mut chars = rest.chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => {
                                if let Some(n) = chars.next() {
                                    s.push(match n {
                                        'n' => '\n',
                                        o => o,
                                    });
                                }
                            }
                            '"' => break,
                            o => s.push(o),
                        }
                    }
                    out.push(s);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_po() {
        let m = parse(
            "msgid \"\"\nmsgstr \"header\"\n\nmsgid \"Hello\"\nmsgstr \"Hallo\"\n\nmsgid \"Two\"\n\"lines\"\nmsgstr \"Zwei \"\n\"Zeilen \\\"x\\\"\"\n",
        );
        assert_eq!(m.get("Hello"), Some(&"Hallo"));
        assert_eq!(m.get("Twolines"), Some(&"Zwei Zeilen \"x\""));
    }

    #[test]
    fn fills() {
        assert_eq!(fill("Live in {} and {}", &["Discord", "OBS"]), "Live in Discord and OBS");
    }

    /// Every text the app shows must have a German translation.
    #[test]
    fn german_is_complete() {
        let de = parse(include_str!("../po/de.po"));
        let mut missing = Vec::new();
        let mut check = |s: &str| {
            if !s.is_empty() && !de.contains_key(s) && !missing.contains(&s.to_string()) {
                missing.push(s.to_string());
            }
        };
        for s in crate::i18n::source_strings() {
            check(&s);
        }
        for v in crate::voice::builtin() {
            check(&v.name);
            check(&v.description);
        }
        for c in crate::voice::CATEGORIES {
            check(c.label);
        }
        for b in crate::dsp::blocks::BLOCKS {
            check(b.name);
            check(b.about);
            for p in b.params {
                check(p.label);
                if let Some(list) = p.choices() {
                    for c in list {
                        check(c);
                    }
                }
            }
        }
        for g in crate::dsp::blocks::Group::ALL {
            check(g.label());
        }
        for s in crate::soundboard::starter::BUILTIN {
            check(s.name);
        }
        for m in crate::soundboard::PlayMode::ALL {
            check(m.label());
            check(m.short());
        }
        for (_, desc, _) in crate::shortcuts::ACTIONS {
            check(desc);
        }
        for (_, _, title) in crate::ui::window::PAGES {
            check(title);
        }
        assert!(missing.is_empty(), "missing German texts:\n{}", missing.join("\n"));
    }
}
