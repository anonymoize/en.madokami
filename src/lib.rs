#![no_std]

use aidoku::{
    Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeLayout, Listing,
    ListingProvider, Manga, MangaPageResult, MangaStatus, Page, PageContent, Result, Source,
    alloc::{String, Vec, string::ToString},
    imports::{defaults::defaults_get, net::*},
    helpers::uri::encode_uri,
    prelude::*,
};
use base64::{engine::general_purpose, Engine as _};
use core::fmt::Write as _; // for simple string building

// Lightweight percent-decoder (handles %XX and + -> space)
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] as char {
            '%' if i + 2 < bytes.len() => {
                let h = (bytes[i+1] as char, bytes[i+2] as char);
                if let (Some(hi), Some(lo)) = (hex(h.0), hex(h.1)) { out.push(((hi << 4) | lo) as char); i += 3; } else { out.push('%'); i += 1; }
            }
            '+' => { out.push(' '); i += 1; }
            c => { out.push(c); i += 1; }
        }
    }
    out
}

fn hex(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(c as u8 - b'0'),
        'a'..='f' => Some(10 + c as u8 - b'a'),
        'A'..='F' => Some(10 + c as u8 - b'A'),
        _ => None,
    }
}

// Encode strictly for query component (space -> %20, etc.)
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-'|'_'|'.'|'~') { out.push(c); } else {
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}

// Derive title & description from path segments (skip leading empty, skip '!' segments for title) replicating Tachiyomi logic
fn derive_from_path(path: &str) -> (String, Option<String>) {
    let segs: Vec<&str> = path.split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() { return (String::new(), None); }
    // description = last segment decoded
    let description = percent_decode(segs.last().unwrap());
    // Walk backwards until a segment not starting with '!'
    let mut title = String::new();
    for seg in segs.iter().rev() {
        let dec = percent_decode(seg);
        if !dec.starts_with('!') { title = dec; break; }
    }
    (title, Some(description))
}

// Normalize reader path: ensure stored chapter key starts with the original anchor href (already contains /reader or needs prefixing) & always relative (leading '/').
fn normalize_chapter_href(raw: &str) -> String {
    if raw.starts_with('/') { raw.to_string() } else { format!("/{}", raw) }
}

// Parse relative date strings like "5 min ago" or absolute format yyyy-MM-dd HH:mm.
fn parse_chapter_date(raw: &str) -> i64 {
    if raw.is_empty() { return 0; }
    if raw.ends_with("ago") {
        let parts: Vec<&str> = raw.split(' ').collect();
        if parts.len() >= 2 { if let Ok(amount) = parts[0].parse::<i64>() {
            return simulate_relative(parts[1], amount);
        }}
        return 0;
    }
    // Absolute date: yyyy-MM-dd HH:mm (naive parsing)
    if raw.len() >= 16 { // 16 = 10 date + 1 space + 5 time
        let year = raw[0..4].parse::<i32>().unwrap_or(1970);
        let month = raw[5..7].parse::<i32>().unwrap_or(1);
        let day = raw[8..10].parse::<i32>().unwrap_or(1);
        let hour = raw[11..13].parse::<i32>().unwrap_or(0);
        let minute = raw[14..16].parse::<i32>().unwrap_or(0);
        // crude unix time approximation: convert to minutes ignoring leap seconds; not perfect but stable.
        // days since epoch (naive) * 86400 + hour*3600 + minute*60
        let days = days_since_epoch(year, month, day);
        return days as i64 * 86400 + hour as i64 * 3600 + minute as i64 * 60;
    }
    0
}

fn simulate_relative(unit: &str, amount: i64) -> i64 {
    // We cannot access current time reliably in no_std; return relative offset as negative seconds from pseudo-now (0).
    // Aidoku may adjust; using 0 - delta gives ordering semantics.
    let secs = if unit.starts_with("min") { amount * 60 }
        else if unit.starts_with("hour") { amount * 3600 }
        else if unit.starts_with("sec") { amount }
        else { 0 };
    -secs
}

fn days_since_epoch(y: i32, m: i32, d: i32) -> i32 { // Gregorian calendar simple calc
    // Source: civil date to days from epoch algorithm (public domain adaptation).
    let y = y - (m <= 2) as i32;
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + (if m > 2 { -3 } else { 9 })) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468 // days since 1970-01-01
}

const BASE_URL: &str = "https://manga.madokami.al";

// =================================================================================
// AUTHENTICATED REQUEST
// =================================================================================
fn auth_get(url: &str) -> Result<Request> {
    let username = defaults_get::<String>("username").unwrap_or_default();
    let password = defaults_get::<String>("password").unwrap_or_default();
    let mut req = Request::get(url)?;
    if !username.is_empty() || !password.is_empty() {
        let encoded = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
        req.set_header("Authorization", &format!("Basic {}", encoded));
    }
    Ok(req)
}

// =================================================================================
// SOURCE IMPLEMENTATION
// =================================================================================
struct Madokami;

impl Source for Madokami {
    fn new() -> Self { Self }

    fn get_search_manga_list(
        &self,
        query: Option<String>,
        _page: i32,
        _filters: Vec<FilterValue>,
    ) -> Result<MangaPageResult> {
        let q = query.unwrap_or_default();
        let url = format!("{BASE_URL}/search?q={}", encode_uri(q));
        let html = auth_get(&url)?.html()?;
        let entries = html
            .select("div.container table tbody tr")
            .map(|rows| {
                rows.filter_map(|row| {
                    let link = row.select_first("td:nth-child(1) a:nth-child(1)")?;
                    let key = link.attr("href")?;
                    let (title, description) = derive_from_path(&key);
                    if title.is_empty() { return None; }
                    Some(Manga { key, title, description: description.filter(|d| !d.is_empty()), ..Default::default() })
                })
                .collect::<Vec<Manga>>()
            })
            .unwrap_or_default();

        Ok(MangaPageResult { entries, has_next_page: false })
    }

    fn get_manga_update(&self, mut manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        let url = format!("{BASE_URL}{}", manga.key);
        let html = auth_get(&url)?.html()?;

        if needs_details {
            manga.cover = html.select("div.manga-info img[itemprop='image']")
                .and_then(|els| els.first())
                .and_then(|el| el.attr("src"));
            // Re-derive title/description from key if not already set
            if manga.title.is_empty() {
                let (title, desc) = derive_from_path(&manga.key);
                if !title.is_empty() { manga.title = title; }
                if manga.description.is_none() { manga.description = desc; }
            }
            if let Some(title_override) = html.select("div.manga-info-title h1").and_then(|el| el.text()) {
                if !title_override.is_empty() { manga.title = title_override; }
            }
            manga.authors = html.select("a[itemprop='author']").map(|els| {
                els.filter_map(|e| e.text()).collect::<Vec<String>>()
            });
            manga.artists = html.select("a[itemprop='artist']").map(|els| {
                els.filter_map(|e| e.text()).collect::<Vec<String>>()
            });
            manga.description = html
                .select("div.manga-info-synopsis")
                .and_then(|el| el.text());
            let status_text = html
                .select("span.scanstatus")
                .and_then(|el| el.text())
                .unwrap_or_default();
            manga.status = match status_text.as_str() {
                "Yes" => MangaStatus::Completed,
                "No" => MangaStatus::Ongoing,
                _ => MangaStatus::Unknown,
            };
            manga.tags = html.select("div.genres a.tag").map(|els| {
                els.filter_map(|e| e.text()).collect::<Vec<String>>()
            });
        }

        if needs_chapters {
            manga.chapters = html.select("table#index-table > tbody > tr").map(|rows| {
                rows.filter_map(|row| {
                    let link = row.select_first("td:nth-child(6) a")?;
                    let href = link.attr("href")?;
                    let key = normalize_chapter_href(&href);
                    let title = row.select_first("td:nth-child(1) a").and_then(|a| a.text());
                    let date_raw = row.select_first("td:nth-child(3)").and_then(|d| d.text()).unwrap_or_default();
                    let date_uploaded = parse_chapter_date(&date_raw);
                    let chapter_num = title
                        .as_ref()
                        .and_then(|t| t.split(' ').find_map(|s| s.parse::<f32>().ok()))
                        .unwrap_or(-1.0);
                    Some(Chapter { key, title, chapter_number: Some(chapter_num), date_uploaded: Some(date_uploaded), ..Default::default() })
                })
                .rev()
                .collect::<Vec<Chapter>>()
            });
        }

        Ok(manga)
    }

    fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{BASE_URL}{}", chapter.key);
        let html = auth_get(&url)?.html()?;
        let (data_path, files_json) = if let Some(el) = html.select("div#reader").and_then(|els| els.first()) {
            (el.attr("data-path").unwrap_or_default(), el.attr("data-files").unwrap_or_default())
        } else { (String::new(), String::new()) };
        if data_path.is_empty() || files_json.is_empty() { return Ok(Vec::new()); }
        let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();
        let pages = files.into_iter().map(|file| {
            let page_url = format!(
                "{BASE_URL}/reader/image?path={}&file={}",
                encode_component(&data_path),
                encode_component(&file)
            );
            Page { content: PageContent::url(page_url), ..Default::default() }
        }).collect::<Vec<Page>>();
        Ok(pages)
    }
}

// =================================================================================
// LISTING PROVIDER
// =================================================================================
impl ListingProvider for Madokami {
    fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
        if listing.id == "recent" {
            let url = format!("{BASE_URL}/recent?page={}", page);
            let html = auth_get(&url)?.html()?;
            let entries = html
                .select("table.mobile-files-table tbody tr")
                .map(|rows| {
                    rows.filter_map(|row| {
                        let link = row.select_first("td:nth-child(1) a:nth-child(1)")?;
                        let key = link.attr("href")?;
                        let (title, description) = derive_from_path(&key);
                        if title.is_empty() { return None; }
                        Some(Manga { key, title, description: description.filter(|d| !d.is_empty()), ..Default::default() })
                    }).collect::<Vec<Manga>>()
                }).unwrap_or_default();
            let has_next_page = html
                .select("a.pagination-next")
                .map(|els| els.filter_map(|_| Some(())).next().is_some())
                .unwrap_or(false);
            Ok(MangaPageResult { entries, has_next_page })
        } else {
            bail!("Unimplemented listing")
        }
    }
}

// =================================================================================
// HOME & DEEPLINK
// =================================================================================
impl Home for Madokami {
    fn get_home(&self) -> Result<HomeLayout> { Ok(HomeLayout::default()) }
}

impl DeepLinkHandler for Madokami {
    fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
        if !url.starts_with(BASE_URL) { return Ok(None); }
        let key = &url[BASE_URL.len()..];
        if key.starts_with("reader/") || key.contains("/reader/") {
            // Could attempt to split manga vs chapter; minimal: treat as chapter
            return Ok(Some(DeepLinkResult::Chapter { manga_key: String::new(), key: key.into() }))
        }
        Ok(Some(DeepLinkResult::Manga { key: key.into() }))
    }
}

// =================================================================================
// REGISTER SOURCE
// =================================================================================
register_source!(Madokami, ListingProvider, Home, DeepLinkHandler);