//! Finds sounds online on MyInstants. It has no API, so this reads its search page, and the page
//! of each sound found for its likes. Only the words someone searches for go out, and only when
//! they search.

use std::time::Duration;

const SITE: &str = "https://www.myinstants.com";
/// Far more than any soundboard sound needs.
const MAX_BYTES: u64 = 15 * 1024 * 1024;

/// One sound the search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub title: String,
    pub url: String,
    /// File type: mp3, ogg, flac or wav.
    pub ext: String,
    /// Its own page on MyInstants, which shows the likes.
    pub page: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No connection, or the server had a problem.
    Offline,
    /// The site wants fewer requests for a while.
    TooMany,
    /// Something else, like a file that does not play.
    Failed,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // A bad connection shows an error soon, instead of a spinner that never ends.
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent(concat!("Voicenux/", env!("CARGO_PKG_VERSION"), " (+https://github.com/voicenux/voicenux)"))
        .build()
        .into()
}

fn error(e: ureq::Error) -> Error {
    log::warn!("sound search: {e}");
    match e {
        ureq::Error::StatusCode(429) => Error::TooMany,
        _ => Error::Offline,
    }
}

/// Sounds for `query`, best first. Blocks: call it off the main thread.
pub fn search(query: &str) -> Result<Vec<Found>, Error> {
    let started = std::time::Instant::now();
    let page = agent().get(format!("{SITE}/en/search/")).query("name", query).call();
    let page = match page {
        // MyInstants answers a search without results with "not found".
        Err(ureq::Error::StatusCode(404)) => String::new(),
        r => r.map_err(error)?.body_mut().read_to_string().map_err(error)?,
    };
    let found = parse(&page);
    log::info!("sound search \"{query}\": {} sounds in {:?}", found.len(), started.elapsed());
    Ok(found)
}

/// The sounds on a search page. Each sits in a `<div class="instant">` with a play button that
/// names the file, and a link with the title.
fn parse(page: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for block in page.split("<div class=\"instant\">").skip(1) {
        let Some(path) = between(block, "play('", "'") else { continue };
        let Some(link) = block.find("instant-link") else { continue };
        let Some(title) = between(&block[link..], ">", "</a>") else { continue };
        let page = between(block, "<a href=\"", "\"")
            .filter(|p| p.starts_with("/en/instant/") && !p.contains("..") && !p.contains(['?', '#']))
            .map(|p| format!("{SITE}{p}"));
        let Some(ext) = path.rsplit_once('.').map(|(_, e)| e.to_lowercase()) else { continue };
        if !path.starts_with("/media/sounds/")
            || path.contains("..")
            || !["mp3", "ogg", "flac", "wav"].contains(&ext.as_str())
        {
            continue;
        }
        let url = format!("{SITE}{}", path.replace(' ', "%20"));
        if found.iter().any(|f| f.url == url) {
            continue;
        }
        found.push(Found { title: clean_title(&unescape(title)), url, ext, page });
    }
    found
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    let len = s[from..].find(end)?;
    Some(&s[from..from + len])
}

/// The few HTML escapes that show up in titles.
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// How many people liked a sound on MyInstants. Blocks: call it off the main thread.
pub fn likes(found: &Found) -> Result<u32, Error> {
    let Some(page) = found.page.as_ref().filter(|p| from_myinstants(p)) else { return Err(Error::Failed) };
    let page = agent().get(page).call().map_err(error)?.body_mut().read_to_string().map_err(error)?;
    Ok(parse_likes(&page))
}

/// The likes on the page of a sound: "<b>3,276 users</b> favorited this sound button". None
/// there yet is 0.
fn parse_likes(page: &str) -> u32 {
    let Some(at) = page.find("id=\"instant-page-likes\"") else { return 0 };
    let Some(count) = between(&page[at..], "<b>", "</b>") else { return 0 };
    let digits: String = count.chars().take_while(|c| !c.is_whitespace()).filter(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// The audio file of a sound. Blocks: call it off the main thread.
pub fn download(found: &Found) -> Result<Vec<u8>, Error> {
    if !from_myinstants(&found.url) {
        return Err(Error::Offline);
    }
    agent().get(&found.url).call().map_err(error)?.body_mut().with_config().limit(MAX_BYTES).read_to_vec().map_err(error)
}

/// Downloads only come from MyInstants, over https.
fn from_myinstants(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host == "www.myinstants.com" || host == "myinstants.com")
}

/// A title as a sound name: "VINE BOOM SOUND" turns into "Vine boom sound", "airhorn_01.wav" into
/// "Airhorn 01".
pub fn clean_title(title: &str) -> String {
    let mut t = title.trim();
    for ext in [".wav", ".mp3", ".ogg", ".flac", ".aif", ".aiff", ".m4a"] {
        // Only cut between letters: titles can be Cyrillic or Japanese.
        if let Some(end) = t.len().checked_sub(ext.len()).filter(|&i| i > 0 && t.is_char_boundary(i))
            && t[end..].eq_ignore_ascii_case(ext)
        {
            t = &t[..end];
        }
    }
    let words: Vec<&str> = t.split(|c: char| c == '_' || c.is_whitespace()).filter(|w| !w.is_empty()).collect();
    let mut name: String = words.join(" ").chars().take(40).collect();
    // All caps shouts on a pad.
    if !name.chars().any(char::is_lowercase) {
        name = name.to_lowercase();
    }
    if let Some(first) = name.chars().next() {
        name = first.to_uppercase().chain(name.chars().skip(1)).collect();
    }
    if name.trim().is_empty() { "Sound".into() } else { name.trim().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <div class="instant">
<button class="small-button" onclick="play('/media/sounds/vine-boom.mp3', 'loader-81126', 'vine-boom-sound-70972')" title="Play VINE BOOM SOUND sound" type="button"></button>
<a href="/en/instant/vine-boom-sound-70972/" class="instant-link link-secondary">VINE BOOM SOUND</a>
        </div>
        <div class="instant">
<button class="small-button" onclick="play('/media/sounds/bruh.mp3', 'loader-2', 'bruh-2')" type="button"></button>
<a href="/en/instant/bruh-2/" class="instant-link link-secondary">Tom &amp; Jerry&#39;s bruh</a>
        </div>
        <div class="instant">
<button class="small-button" onclick="play('/media/sounds/../../etc/passwd.mp3', 'l', 'x')"></button>
<a class="instant-link">Sneaky</a>
        </div>
        <div class="instant">
<button class="small-button" onclick="play('/media/sounds/clip.webm', 'l', 'x')"></button>
<a class="instant-link">Video</a>
        </div>"#;

    #[test]
    fn reads_the_search_page() {
        let found = parse(PAGE);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "Vine boom sound");
        assert_eq!(found[0].url, "https://www.myinstants.com/media/sounds/vine-boom.mp3");
        assert_eq!(found[0].ext, "mp3");
        assert_eq!(found[1].title, "Tom & Jerry's bruh");
        assert_eq!(found[0].page.as_deref(), Some("https://www.myinstants.com/en/instant/vine-boom-sound-70972/"));
    }

    #[test]
    fn reads_the_likes() {
        let page = r#"<div id="instant-page-likes">
      <img src="/media/images/icons/favorite-red.svg" alt="Favorites">
      <b>3,276 users</b>
      favorited this sound button</div>"#;
        assert_eq!(parse_likes(page), 3276);
        assert_eq!(parse_likes(&page.replace("3,276", "1")), 1);
        assert_eq!(parse_likes("<html>no likes yet</html>"), 0);
    }

    #[test]
    fn titles_turn_into_names() {
        assert_eq!(clean_title("airhorn_01.wav"), "Airhorn 01");
        assert_eq!(clean_title("  WOW male voice.MP3 "), "WOW male voice");
        assert_eq!(clean_title("BRUH!!"), "Bruh!!");
        assert_eq!(clean_title("шшш"), "Шшш");
        assert_eq!(clean_title("日本"), "日本");
        assert_eq!(clean_title("Звук.mp3"), "Звук");
        assert_eq!(clean_title(""), "Sound");
        assert_eq!(clean_title(&"a".repeat(80)).chars().count(), 40);
    }

    #[test]
    fn downloads_only_from_myinstants() {
        assert!(from_myinstants("https://www.myinstants.com/media/sounds/vine-boom.mp3"));
        assert!(!from_myinstants("http://www.myinstants.com/media/sounds/vine-boom.mp3"));
        assert!(!from_myinstants("https://www.myinstants.com.evil.example/1.mp3"));
        assert!(!from_myinstants("https://example.com/www.myinstants.com/1.mp3"));
    }

    /// Talks to the real MyInstants, so only on request: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn finds_and_downloads_a_sound() {
        let _ = env_logger::builder().is_test(true).try_init();
        let found = search("vine boom").unwrap();
        assert!(!found.is_empty());
        assert!(likes(&found[0]).unwrap() > 0);
        let bytes = download(&found[0]).unwrap();
        assert!(super::super::decode_bytes(bytes, &found[0].ext).is_ok());
        assert!(search("qwxzzk").unwrap().is_empty());
        // Titles in other scripts once crashed the search.
        for q in ["привет", "шш", "日本", "ñ"] {
            search(q).unwrap();
        }
    }
}
