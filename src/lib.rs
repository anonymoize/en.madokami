#![no_std]

use aidoku::{
    Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeLayout, Listing,
    ListingProvider, Manga, MangaPageResult, MangaStatus, Page, PageContent, Result, Source,
    alloc::{String, Vec},
    imports::{defaults::defaults_get, net::*},
    helpers::uri::encode_uri,
    prelude::*,
};
use base64::{engine::general_purpose, Engine as _};

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
                    let title = link.text()?;
                    Some(Manga { key, title, ..Default::default() })
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
            if let Some(title) = html.select("div.manga-info-title h1").and_then(|el| el.text()) {
                manga.title = title;
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
                    let key = row.select_first("td:nth-child(6) a")?.attr("href")?;
                    let title = row.select_first("td:nth-child(1) a").and_then(|a| a.text());
                    let chapter_num = title
                        .as_ref()
                        .and_then(|t| t.split(' ').find_map(|s| s.parse::<f32>().ok()))
                        .unwrap_or(-1.0);
                    Some(Chapter { key, title, chapter_number: Some(chapter_num), ..Default::default() })
                })
                .rev() // site lists newest first; reverse to oldest->newest
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
        let pages = files
            .into_iter()
            .map(|file| {
                let page_url = format!(
                    "{BASE_URL}/reader/image?path={}&file={}",
                    encode_uri(data_path.clone()),
                    encode_uri(file)
                );
                Page { content: PageContent::url(page_url), ..Default::default() }
            })
            .collect::<Vec<Page>>();
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
                        let title = link.text()?;
                        Some(Manga { key, title, ..Default::default() })
                    })
                    .collect::<Vec<Manga>>()
                })
                .unwrap_or_default();
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
        Ok(Some(DeepLinkResult::Manga { key: key.into() }))
    }
}

// =================================================================================
// REGISTER SOURCE
// =================================================================================
register_source!(Madokami, ListingProvider, Home, DeepLinkHandler);