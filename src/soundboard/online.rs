//! Finds sounds online on MyInstants. It has no API, so this reads its front page and its search
//! page, and the page of each sound for its likes and views. Nothing goes out before you open Find
//! sounds; then only your country for the front page and the words you search for.

use std::sync::LazyLock;
use std::time::Duration;

use ureq::config::IpFamily;

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
    /// Its own page on MyInstants, which shows the likes and views.
    pub page: Option<String>,
    /// MyInstants numbers sounds as they come in: a higher one is newer. 0 when unknown.
    pub id: u32,
}

/// What MyInstants shows on the page of a sound.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// People who liked it.
    pub likes: u32,
    /// How often its page was opened. MyInstants counts no downloads, this is the closest.
    pub views: u32,
    /// What its uploader wrote about it, and its tags. Often empty.
    pub about: String,
    /// Its category in lowercase, like "pranks" or "sound effects". Empty for none.
    pub category: String,
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

/// One agent for all requests, so they share their connections. IPv4 first: where IPv6 is set
/// up but does not reach out, trying it first took 3 s for every request. Hosts without IPv4 are
/// asked over both.
static V4: LazyLock<ureq::Agent> = LazyLock::new(|| agent(IpFamily::Ipv4Only));
static ANY: LazyLock<ureq::Agent> = LazyLock::new(|| agent(IpFamily::Any));

fn agent(family: IpFamily) -> ureq::Agent {
    ureq::Agent::config_builder()
        .ip_family(family)
        // A bad connection shows an error soon, instead of a spinner that never ends.
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(12)))
        .user_agent(concat!("Voicenux/", env!("CARGO_PKG_VERSION"), " (+https://github.com/voicenux/voicenux)"))
        .build()
        .into()
}

type Response = ureq::http::Response<ureq::Body>;

fn call(request: impl Fn(&ureq::Agent) -> Result<Response, ureq::Error>) -> Result<Response, ureq::Error> {
    match request(&V4) {
        Err(ureq::Error::HostNotFound) => request(&ANY),
        r => r,
    }
}

fn error(e: ureq::Error) -> Error {
    log::warn!("sound search: {e}");
    match e {
        ureq::Error::StatusCode(429) => Error::TooMany,
        _ => Error::Offline,
    }
}

/// Sounds for `query`, best first, page `page` counted from 1. Past the last page there are
/// none. Blocks: call it off the main thread.
pub fn search(query: &str, page: u32) -> Result<Vec<Found>, Error> {
    let started = std::time::Instant::now();
    let text =
        read(|a| a.get(format!("{SITE}/en/search/")).query("name", query).query("page", page.to_string()).call())?;
    let found = parse(&text);
    log::info!("sound search \"{query}\" page {page}: {} sounds in {:?}", found.len(), started.elapsed());
    Ok(found)
}

/// The page a request answers with. MyInstants answers a search without results, or a page past
/// the last one, with "not found": that is an empty page.
fn read(request: impl Fn(&ureq::Agent) -> Result<Response, ureq::Error>) -> Result<String, Error> {
    match call(request) {
        Err(ureq::Error::StatusCode(404)) => Ok(String::new()),
        r => r.map_err(error)?.body_mut().read_to_string().map_err(error),
    }
}

/// The sounds on the front page of MyInstants for your country, most played first, page `page`
/// counted from 1. Blocks: call it off the main thread.
pub fn popular(page: u32) -> Result<Vec<Found>, Error> {
    let mut last = Ok(Vec::new());
    for region in [region(), "us".into()] {
        last = read(|a| a.get(format!("{SITE}/en/index/{region}/")).query("page", page.to_string()).call())
            .map(|text| parse(&text));
        if last.as_ref().is_ok_and(|f| !f.is_empty()) {
            break;
        }
    }
    last
}

/// The country of the system language, like "de" for de_DE.UTF-8. MyInstants has a front page
/// for each.
fn region() -> String {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.is_empty());
    lang.and_then(|l| {
        let country = l.split(['.', '@']).next()?.split_once('_')?.1.to_lowercase();
        (country.len() == 2 && country.chars().all(|c| c.is_ascii_lowercase())).then_some(country)
    })
    .unwrap_or_else(|| "us".into())
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
        let id = between(block, "'loader-", "'").and_then(|n| n.parse().ok()).unwrap_or(0);
        found.push(Found { title: clean_title(&unescape(title)), url, ext, page, id });
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

/// The likes and views of a sound on MyInstants. Blocks: call it off the main thread.
pub fn stats(found: &Found) -> Result<Stats, Error> {
    let Some(page) = found.page.as_ref().filter(|p| from_myinstants(p)) else { return Err(Error::Failed) };
    let page = call(|a| a.get(page).call()).map_err(error)?.body_mut().read_to_string().map_err(error)?;
    Ok(Stats {
        likes: parse_likes(&page),
        views: parse_views(&page),
        about: parse_about(&page),
        category: parse_category(&page),
    })
}

/// The sound for `query` that most people liked, out of the first ones the search finds, with
/// its numbers when MyInstants gave them. None when it finds nothing. Blocks: call it off the
/// main thread.
pub fn most_liked(query: &str) -> Result<Option<(Found, Option<Stats>)>, Error> {
    // The search puts the best ones first anyway. Asking for more pages only takes longer.
    const LOOK_AT: usize = 8;
    let found: Vec<Found> = search(query, 1)?.into_iter().take(LOOK_AT).collect();
    // Four at a time, not all at once: MyInstants turns floods away.
    let stats: Vec<Option<Stats>> = found
        .chunks(4)
        .flat_map(|four| {
            std::thread::scope(|s| {
                let asks: Vec<_> = four.iter().map(|f| s.spawn(move || stats(f).ok())).collect();
                asks.into_iter().map(|a| a.join().ok().flatten()).collect::<Vec<_>>()
            })
        })
        .collect();
    let likes = |i: usize| stats[i].as_ref().map_or(0, |s| s.likes);
    // The first one wins a tie.
    let best = (0..found.len()).max_by_key(|&i| (likes(i), std::cmp::Reverse(i)));
    Ok(best.map(|i| (found[i].clone(), stats[i].clone())))
}

/// What the uploader wrote and the tags: `<div id="instant-page-description"><p>Brain Meme</p>`
/// and `<div id="instant-page-tags"><a href="/en/search/?name=x">#x</a>`.
fn parse_about(page: &str) -> String {
    let mut about = String::new();
    for id in ["instant-page-description", "instant-page-tags"] {
        let Some(at) = page.find(&format!("id=\"{id}\"")) else { continue };
        let Some(part) = between(&page[at..], ">", "</div>") else { continue };
        let mut inside = false;
        for c in part.chars() {
            match c {
                '<' => inside = true,
                '>' => {
                    inside = false;
                    about.push(' ');
                }
                c if !inside => about.push(c),
                _ => {}
            }
        }
        about.push(' ');
    }
    unescape(&about.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The category in the breadcrumbs: `<a href="/en/categories/sound%20effects/"`. Lowercase.
fn parse_category(page: &str) -> String {
    let Some(at) = page.find("id=\"breadcrumbs\"") else { return String::new() };
    let crumbs = &page[at..];
    let crumbs = &crumbs[..crumbs.find("</div>").unwrap_or(crumbs.len())];
    between(crumbs, "/en/categories/", "/").map(|c| unescape(&c.replace("%20", " ")).to_lowercase()).unwrap_or_default()
}

/// The views on the page of a sound: "<div>11,659,629 views</div>". None there is 0.
fn parse_views(page: &str) -> u32 {
    let Some(end) = page.find(" views</div>") else { return 0 };
    let start = page[..end].rfind('>').map_or(0, |i| i + 1);
    let digits: String = page[start..end].chars().filter(char::is_ascii_digit).collect();
    // More than u32 holds: as many as it can.
    if digits.is_empty() { 0 } else { digits.parse().unwrap_or(u32::MAX) }
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
    call(|a| a.get(&found.url).call())
        .map_err(error)?
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(error)
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
        assert_eq!(found[0].id, 81126);
        assert_eq!(found[1].id, 2);
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
    fn reads_the_views() {
        let page = r#"favorited this sound button </div> <div>11,659,629 views</div> <div id="x">"#;
        assert_eq!(parse_views(page), 11_659_629);
        assert_eq!(parse_views("<div>1 views</div>"), 1);
        assert_eq!(parse_views("<html>new sound</html>"), 0);
    }

    #[test]
    fn reads_what_the_page_says_about_a_sound() {
        let page = r#"<div id="breadcrumbs"><ol>
      <li><a href="/en/search/" title="Sounds" class="link-secondary">Sounds</a> ></li>
      <li><a href="/en/categories/anime%20&amp;%20manga/" title="Anime &amp; Manga">Anime &amp; Manga</a> ></li>
      <li><span>Galaxy meme</span></li></ol></div>
      <h1 id="instant-page-title">Galaxy meme</h1>
      <div id="instant-page-description">
        <p>Brain &amp; Meme</p>
      </div>
      <div id="instant-page-tags">
        <a href="/en/search/?name=fart">#fart</a>
      </div>"#;
        assert_eq!(parse_category(page), "anime & manga");
        assert_eq!(parse_about(page), "Brain & Meme #fart");
        assert_eq!(parse_category("<html>none</html>"), "");
        assert_eq!(parse_about("<html>none</html>"), "");
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
        let found = search("vine boom", 1).unwrap();
        assert!(!found.is_empty());
        let stats = stats(&found[0]).unwrap();
        assert!(stats.likes > 0 && stats.views > stats.likes);
        assert!(popular(1).unwrap().len() > 10);
        let (best, stats) = most_liked("vine boom").unwrap().unwrap();
        assert!(stats.is_some_and(|s| s.likes > 0), "{best:?}");
        assert!(most_liked("qwxzzk").unwrap().is_none());
        let more = search("vine boom", 2).unwrap();
        assert!(!more.is_empty() && more[0] != found[0]);
        assert!(search("vine boom", 999).unwrap().is_empty());
        let bytes = download(&found[0]).unwrap();
        assert!(super::super::decode_bytes(bytes, &found[0].ext).is_ok());
        assert!(search("qwxzzk", 1).unwrap().is_empty());
        // Titles in other scripts once crashed the search.
        for q in ["привет", "шш", "日本", "ñ"] {
            search(q, 1).unwrap();
        }
    }
}
