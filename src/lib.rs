#![no_std]
extern crate alloc;

// Corrected Imports
use aidoku::{
    error::{Result, AidokuError},
    prelude::*,
    std::net::{Request, Method},
    std::{String, Vec},
    helpers::uri::encode_uri,
    Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeLayout, Listing, Manga,
    MangaPageResult, MangaStatus, Page, PageContent, Source, ListingProvider,
};
use alloc::string::ToString;
use base64::{Engine as _, engine::general_purpose};

const BASE_URL: &str = "https://manga.madokami.al";

// =================================================================================
// AUTHENTICATION HELPER
// =================================================================================

fn authenticated_request(url: &str, method: Method) -> Request {
    let username = aidoku::imports::defaults::defaults_get::<String>("username").unwrap_or_default();
    let password = aidoku::imports::defaults::defaults_get::<String>("password").unwrap_or_default();
    let credentials = format!("{}:{}", username, password);
    let encoded_credentials = general_purpose::STANDARD.encode(credentials);
    Request::new(url.to_string(), method)
        .header("Authorization".to_string(), format!("Basic {}", encoded_credentials))
}

// =================================================================================
// SOURCE IMPLEMENTATION
// =================================================================================

#[derive(Default)]
struct Madokami;

impl Source for Madokami {
    fn new() -> Self {
        Self
    }

    fn get_search_manga_list(
        &self,
        query: Option<String>,
        _page: i32,
        _filters: Vec<FilterValue>,
    ) -> Result<MangaPageResult> {
        let query = query.unwrap_or_default();
        let url = format!("{}/search?q={}", BASE_URL, encode_uri(query));
        let html = authenticated_request(&url, Method::Get).html()?;
        let mut manga: Vec<Manga> = Vec::new();

        for item in html.select("div.container table tbody tr").array() {
            let manga_node = item.as_node()?;
            let title_element = manga_node.select("td:nth-child(1) a:nth-child(1)");
            let key = title_element.attr("href").read();
            let title = title_element.text().read();

            manga.push(Manga {
                key,
                title,
                ..Default::default()
            });
        }

        Ok(MangaPageResult {
            entries: manga,
            has_next_page: false,
        })
    }

    fn get_manga_update(&self, mut manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        let url = format!("{}{}", BASE_URL, manga.key);
        let html = authenticated_request(&url, Method::Get).html()?;

        if needs_details {
            manga.cover = Some(html.select("div.manga-info img[itemprop='image']").attr("src").read());
            manga.title = html.select("div.manga-info-title h1").text().read();
            manga.authors = Some(html.select("a[itemprop='author']").array().map(|elem| elem.as_node().unwrap().text().read()).collect::<Vec<String>>());
            manga.artists = Some(html.select("a[itemprop='artist']").array().map(|elem| elem.as_node().unwrap().text().read()).collect::<Vec<String>>());
            manga.description = Some(html.select("div.manga-info-synopsis").text().read());
            let status_text = html.select("span.scanstatus").text().read();
            manga.status = match status_text.as_str() {
                "Yes" => MangaStatus::Completed,
                "No" => MangaStatus::Ongoing,
                _ => MangaStatus::Unknown,
            };
            manga.tags = Some(html.select("div.genres a.tag").array().map(|elem| elem.as_node().unwrap().text().read()).collect::<Vec<String>>());
        }

        if needs_chapters {
            let mut chapters: Vec<Chapter> = Vec::new();
            for item in html.select("table#index-table > tbody > tr").array() {
                let chapter_node = item.as_node()?;
                let key = chapter_node.select("td:nth-child(6) a").attr("href").read();
                let title = chapter_node.select("td:nth-child(1) a").text().read();
                let chapter_num = title.split(' ').find_map(|s| s.parse::<f32>().ok()).unwrap_or(-1.0);

                chapters.push(Chapter {
                    key,
                    title: Some(title),
                    chapter_number: Some(chapter_num),
                    ..Default::default()
                });
            }
            chapters.reverse();
            manga.chapters = Some(chapters);
        }

        Ok(manga)
    }

    fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}{}", BASE_URL, chapter.key);
        let html = authenticated_request(&url, Method::Get).html()?;
        let reader_div = html.select("div#reader").first().expect("Reader div not found");
        let path = reader_div.attr("data-path").read();
        let files_json = reader_div.attr("data-files").read();
        let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();
        let mut pages: Vec<Page> = Vec::new();

        for file in files.iter() {
            let page_url = format!("{}/reader/image?path={}&file={}", BASE_URL, encode_uri(path.clone()), encode_uri(file.clone()));
            pages.push(Page {
                content: PageContent::url(page_url),
                ..Default::default()
            });
        }

        Ok(pages)
    }
}

// =================================================================================
// LISTING PROVIDER
// =================================================================================
impl ListingProvider for Madokami {
    fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
        if listing.id == "recent" {
            let url = format!("{}/recent?page={}", BASE_URL, page);
            let html = authenticated_request(&url, Method::Get).html()?;
            let mut manga: Vec<Manga> = Vec::new();

            for item in html.select("table.mobile-files-table tbody tr").array() {
                let manga_node = item.as_node()?;
                let title_element = manga_node.select("td:nth-child(1) a:nth-child(1)");
                let key = title_element.attr("href").read();
                let title = title_element.text().read();

                manga.push(Manga {
                    key,
                    title,
                    ..Default::default()
                });
            }

            let has_next_page = !html.select("a.pagination-next").array().is_empty();

            Ok(MangaPageResult {
                entries: manga,
                has_next_page,
            })
        } else {
            Err(AidokuError::Unimplemented)
        }
    }
}

// =================================================================================
// HOME & DEEPLINK
// =================================================================================
impl Home for Madokami {
    fn get_home(&self) -> Result<HomeLayout> {
        Ok(HomeLayout::default())
    }
}

impl DeepLinkHandler for Madokami {
    fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
        if url.starts_with(BASE_URL) {
            let key = url.replace(BASE_URL, "");
            Ok(Some(DeepLinkResult::Manga { key }))
        } else {
            Ok(None)
        }
    }
}

// =================================================================================
// REGISTER SOURCE
// =================================================================================
register_source!(Madokami, ListingProvider, Home, DeepLinkHandler);