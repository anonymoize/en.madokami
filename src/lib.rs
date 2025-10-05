#![no_std]
use aidoku::{
	error::{Result, AidokuError},
	prelude::*,
	std::{
		net::{Request, Method},
		String, Vec,
	},
	helpers::uri::encode_uri,
	helpers::setting::get_string,
	Chapter, DeepLink, DeepLinkResult, Filter, FilterType, Home, HomeLayout, Listing, Manga,
	MangaContentRating, MangaPageResult, MangaStatus, MangaViewer, Page,
};

const BASE_URL: &str = "https://manga.madokami.al";

// =================================================================================
// AUTHENTICATION HELPER
// =================================================================================

/// Creates a request with the necessary Basic Authentication headers.
fn authenticated_request(url: &str, method: Method) -> Request {
	let username = get_string("username").unwrap_or_default();
	let password = get_string("password").unwrap_or_default();
	let credentials = format!("{}:{}", username, password);
	let encoded_credentials = base64::encode(credentials);
	Request::new(url, method)
		.header("Authorization", &format!("Basic {}", encoded_credentials))
}


// =================================================================================
// SOURCE IMPLEMENTATION
// =================================================================================

#[derive(Default)]
struct enmadokami;

impl Source for enmadokami {

	/// This function is called when the user performs a search.
	fn get_search_manga_list(&self, filters: Vec<Filter>, _page: i32) -> Result<MangaPageResult> {
		let query = filters.into_iter()
			.find(|filter| filter.kind == FilterType::Title)
			.and_then(|filter| filter.value.as_string())
			.map(|value| value.read())
			.unwrap_or_default();

		let url = format!("{}/search?q={}", BASE_URL, encode_uri(query));
		let html = authenticated_request(&url, Method::Get).get_html()?;
		let mut manga: Vec<Manga> = Vec::new();

		for item in html.select("div.container table tbody tr").array() {
			let manga_node = item.as_node()?;
			let title_element = manga_node.select("td:nth-child(1) a:nth-child(1)");
			let id = title_element.attr("href").read();
			let title = title_element.text().read();

			manga.push(Manga {
				id,
				title,
				..Default::default()
			});
		}

		// Search results on Madokami are not paginated
		Ok(MangaPageResult { manga, has_more: false })
	}

	/// This function is called to get details and chapters for a specific manga.
	fn get_manga_update(&self, manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
		let mut manga = manga;
		let url = format!("{}{}", BASE_URL, manga.id);
		let html = authenticated_request(&url, Method::Get).get_html()?;

		if needs_details {
			manga.cover = html.select("div.manga-info img[itemprop='image']").attr("src").read();
			manga.title = html.select("div.manga-info-title h1").text().read();
			manga.author = html.select("a[itemprop='author']").array()
				.map(|elem| elem.as_node().unwrap().text().read()).collect::<Vec<String>>().join(", ");
			manga.artist = html.select("a[itemprop='artist']").array()
				.map(|elem| elem.as_node().unwrap().text().read()).collect::<Vec<String>>().join(", ");
			manga.description = html.select("div.manga-info-synopsis").text().read();

			let status_text = html.select("span.scanstatus").text().read();
			manga.status = match status_text.as_str() {
				"Yes" => MangaStatus::Completed,
				"No" => MangaStatus::Ongoing,
				_ => MangaStatus::Unknown,
			};

			manga.categories = html.select("div.genres a.tag").array()
				.map(|elem| elem.as_node().unwrap().text().read()).collect::<Vec<String>>();
		}

		if needs_chapters {
			let mut chapters: Vec<Chapter> = Vec::new();
			for item in html.select("table#index-table > tbody > tr").array() {
				let chapter_node = item.as_node()?;
				let url = chapter_node.select("td:nth-child(6) a").attr("href").read();
				let title = chapter_node.select("td:nth-child(1) a").text().read();

				let chapter_num = title.split(' ').find_map(|s| s.parse::<f32>().ok()).unwrap_or(-1.0);

				chapters.push(Chapter {
					id: url,
					title,
					chapter: chapter_num,
					..Default::default()
				});
			}
			chapters.reverse();
			manga.chapters = chapters;
		}

		Ok(manga)
	}

	/// This function is called to get the image URLs for a chapter.
	fn get_page_list(&self, _manga_id: String, chapter_id: String) -> Result<Vec<Page>> {
		let url = format!("{}{}", BASE_URL, chapter_id);
		let html = authenticated_request(&url, Method::Get).get_html()?;
		let reader_div = html.select("div#reader");

		let path = reader_div.attr("data-path").read();
		let files_json = reader_div.attr("data-files").read();

		let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();
		let mut pages: Vec<Page> = Vec::new();

		for (index, file) in files.iter().enumerate() {
			let page_url = format!("{}/reader/image?path={}&file={}", BASE_URL, encode_uri(path.clone()), encode_uri(file.clone()));
			pages.push(Page {
				index: index as i32,
				url: page_url,
				..Default::default()
			});
		}

		Ok(pages)
	}
}


// =================================================================================
// LISTING PROVIDER
// =================================================================================
/// This handles browsing the "Recent" tab on the source's main page.
impl ListingProvider for enmadokami {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		if listing.id == "recent" {
			let url = format!("{}/recent?page={}", BASE_URL, page);
			let html = authenticated_request(&url, Method::Get).get_html()?;
			let mut manga: Vec<Manga> = Vec::new();

			for item in html.select("table.mobile-files-table tbody tr").array() {
				let manga_node = item.as_node()?;
				let title_element = manga_node.select("td:nth-child(1) a:nth-child(1)");
				let id = title_element.attr("href").read();
				let title = title_element.text().read();

				manga.push(Manga {
					id,
					title,
					..Default::default()
				});
			}

			let has_more = !html.select("a.pagination-next").array().is_empty();

			Ok(MangaPageResult { manga, has_more })
		} else {
			Err(AidokuError::Unimplemented)
		}
	}
}


// =================================================================================
// HOME & DEEPLINK (STUBS)
// =================================================================================
// Madokami doesn't have a rich homepage, so we'll return an empty layout.
impl Home for enmadokami {
	fn get_home(&self) -> Result<HomeLayout> {
		Ok(HomeLayout::default())
	}
}

// Madokami URLs are straightforward, but we'll add this for completeness.
impl DeepLinkHandler for enmadokami {
    fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
        // Example: https://manga.madokami.al/Manga/S/SU/SUZU/Suzuki-san-wa-Ichi-Rin-no-Hana/
        // We just need the path part as the manga ID.
		if url.starts_with(BASE_URL) {
			let path = url.replace(BASE_URL, "");
			// We can improve this later to handle chapters specifically if needed
			Ok(Some(DeepLinkResult::Manga { id: path }))
		} else {
			Ok(None)
		}
    }
}

// =================================================================================
// REGISTER SOURCE
// =================================================================================
register_source!(enmadokami, ListingProvider, Home, DeepLinkHandler);

