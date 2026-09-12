//! Content extraction: web pages, YouTube subtitles, PDFs, topic search.

use scraper::{Html, Selector};
use serde_json::Value;
use std::time::Duration;

use crate::error::AppError;

pub struct Extractor {
    http: reqwest::Client,
}

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36 PodcastfyUI/0.1";

impl Default for Extractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent(USER_AGENT)
            .danger_accept_invalid_certs(false)
            .build()
            .expect("http client");
        Self { http }
    }

    /// Dispatch a single URL to the right extractor (YouTube vs. web page).
    /// `output_language` 用于给 YouTube 字幕挑选更贴合的轨道。
    pub async fn extract_url(&self, url: &str, output_language: &str) -> Result<String, AppError> {
        let normalized = url.trim().to_string();
        if is_youtube(&normalized) {
            self.extract_youtube(&normalized, output_language).await
        } else {
            self.extract_webpage(&normalized).await
        }
    }

    /// Fetch a web page and pull the readable main content out of the HTML.
    pub async fn extract_webpage(&self, url: &str) -> Result<String, AppError> {
        let html_text = self
            .http
            .get(url)
            .header("Accept", "text/html,application/xhtml+xml")
            .send()
            .await
            .map_err(|e| AppError::Extraction(format!("fetch {url}: {e}")))?
            .text()
            .await
            .map_err(|e| AppError::Extraction(format!("read body {url}: {e}")))?;

        let parsed = Html::parse_document(&html_text);
        let url_parsed = url::Url::parse(url)
            .map_err(|e| AppError::Extraction(format!("bad url {url}: {e}")))?;
        let mut bytes = html_text.as_bytes();
        let product = readability::extractor::extract(&mut bytes, &url_parsed)
            .map_err(|e| AppError::Extraction(format!("readability {url}: {e}")))?;

        let title = product.title.trim().to_string();
        let cleaned = clean_text(&product.text);
        if cleaned.chars().count() < 40 {
            return Err(AppError::Extraction(format!(
                "extracted content too short for {url} — page may be JS-rendered"
            )));
        }

        // Fallback: if readability gave little, try <article>/<main> selectors.
        // Keep whichever is longer — a page without those tags must not lose
        // the readability result.
        let mut text = if cleaned.chars().count() > 200 {
            cleaned
        } else {
            let sel = Selector::parse("article, main, [role=main]").unwrap();
            let parts: Vec<String> = parsed
                .select(&sel)
                .map(|el| el.text().collect::<Vec<_>>().join(" "))
                .collect();
            let fallback = clean_text(&parts.join("\n"));
            if fallback.chars().count() > cleaned.chars().count() {
                fallback
            } else {
                cleaned
            }
        };

        // Truncate very long pages so LLM context stays bounded.
        let max_chars = 60_000;
        if text.chars().count() > max_chars {
            text = take_chars(&text, max_chars) + "\n\n[truncated]";
        }

        Ok(format!("Title: {title}\nURL: {url}\n\n{text}"))
    }

    /// Fetch YouTube subtitles (captions) for a video URL.
    pub async fn extract_youtube(&self, url: &str, output_language: &str) -> Result<String, AppError> {
        let video_id = youtube_video_id(url)
            .ok_or_else(|| AppError::Extraction(format!("not a youtube URL: {url}")))?;

        // 1) Get the watch page and parse captionTracks from ytInitialPlayerResponse.
        let watch = format!("https://www.youtube.com/watch?v={video_id}");
        let page = self
            .http
            .get(&watch)
            .header("Accept-Language", "en")
            .send()
            .await
            .map_err(|e| AppError::Extraction(format!("fetch youtube: {e}")))?
            .text()
            .await
            .map_err(|e| AppError::Extraction(format!("read youtube page: {e}")))?;

        let player_json = extract_yt_initial(&page)
            .ok_or_else(|| AppError::Extraction("ytInitialPlayerResponse not found".into()))?;
        let tracks = player_json
            .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::Extraction("no caption tracks in player response".into()))?;

        if tracks.is_empty() {
            return Err(AppError::Extraction(
                "this video has no captions/subtitles".into(),
            ));
        }

        // 优先匹配输出语言（中文→语言代码以 zh 开头），其次英文，最后第一条。
        let norm = crate::config::normalize_language(output_language);
        let prefer_zh =
            norm.starts_with("Simplified Chinese") || norm.starts_with("Traditional Chinese");
        let find_by = |prefix: &str| {
            tracks.iter().find(|t| {
                t.pointer("/languageCode")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.starts_with(prefix))
            })
        };
        let track: Value = if prefer_zh {
            find_by("zh").or_else(|| find_by("en"))
        } else {
            find_by("en")
        }
        .or_else(|| tracks.first())
        .cloned()
        .ok_or_else(|| AppError::Extraction("no caption track selected".into()))?;
        let base_url = track
            .get("baseUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Extraction("caption baseUrl missing".into()))?;

        // 2) Fetch the timedtext payload (XML).
        let timedtext = format!("{base_url}&fmt=xml");
        let xml = self
            .http
            .get(&timedtext)
            .send()
            .await
            .map_err(|e| AppError::Extraction(format!("fetch timedtext: {e}")))?
            .text()
            .await
            .map_err(|e| AppError::Extraction(format!("read timedtext: {e}")))?;

        let transcript = parse_timedtext_xml(&xml);
        if transcript.trim().is_empty() {
            return Err(AppError::Extraction("timedtext returned no text".into()));
        }
        let transcript = clean_text(&transcript);

        // Also grab the video title if available.
        let title = player_json
            .pointer("/videoDetails/title")
            .and_then(Value::as_str)
            .unwrap_or("");

        Ok(format!(
            "YouTube video: {title}\nURL: {url}\n\n{transcript}"
        ))
    }

    /// Extract plain text from a local PDF file.
    pub async fn extract_pdf(&self, path: &str) -> Result<String, AppError> {
        // CPU-bound sync work — keep the runtime unblocked.
        let path_owned = path.to_string();
        let text = tokio::task::spawn_blocking(move || pdf_extract::extract_text(&path_owned))
            .await
            .map_err(|e| AppError::Extraction(format!("pdf task join: {e}")))?
            .map_err(|e| AppError::Extraction(format!("pdf {path}: {e}")))?;
        let cleaned = clean_text(&text);
        if cleaned.trim().is_empty() {
            return Err(AppError::Extraction(format!(
                "no extractable text in {path} (scanned image PDF?)"
            )));
        }
        Ok(cleaned)
    }

    /// Expand a topic into background material via the Serper.dev search API.
    ///
    /// If no API key is configured, falls back to DuckDuckGo HTML search
    /// (lower quality, no key needed).
    pub async fn search_topic(
        &self,
        topic: &str,
        serper_api_key: Option<&str>,
    ) -> Result<String, AppError> {
        if let Some(key) = serper_api_key.filter(|k| !k.is_empty()) {
            return self.search_topic_serper(topic, key).await;
        }
        self.search_topic_ddg(topic).await
    }

    async fn search_topic_serper(&self, topic: &str, key: &str) -> Result<String, AppError> {
        let resp = self
            .http
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", key)
            .json(&serde_json::json!({ "q": topic, "num": 5 }))
            .send()
            .await
            .map_err(|e| AppError::Extraction(format!("serper request: {e}")))?;
        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Extraction(format!("serper json: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Extraction(format!(
                "serper error {status}: {}",
                v.get("message").and_then(Value::as_str).unwrap_or("unknown")
            )));
        }
        let results: Vec<String> = v
            .get("organic")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|r| {
                        let title = r.get("title")?.as_str()?.to_string();
                        let snippet = r.get("snippet").and_then(Value::as_str).unwrap_or("");
                        let link = r.get("link").and_then(Value::as_str).unwrap_or("");
                        Some(format!("### {title}\n{snippet}\n({link})"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if results.is_empty() {
            return Err(AppError::Extraction("serper returned no results".into()));
        }
        Ok(results.join("\n\n"))
    }

    async fn search_topic_ddg(&self, topic: &str) -> Result<String, AppError> {
        let html = self
            .http
            .post("https://html.duckduckgo.com/html/")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[("q", topic)])
            .send()
            .await
            .map_err(|e| AppError::Extraction(format!("ddg request: {e}")))?
            .text()
            .await
            .map_err(|e| AppError::Extraction(format!("ddg body: {e}")))?;

        let parsed = Html::parse_document(&html);
        let result_sel = Selector::parse(".result").unwrap();
        let title_sel = Selector::parse(".result__a").unwrap();
        let snippet_sel = Selector::parse(".result__snippet").unwrap();

        let mut parts = Vec::new();
        for r in parsed.select(&result_sel).take(5) {
            let title = r
                .select(&title_sel)
                .next()
                .map(|t| t.text().collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let snippet = r
                .select(&snippet_sel)
                .next()
                .map(|t| t.text().collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            if !title.is_empty() {
                parts.push(format!("### {title}\n{snippet}"));
            }
        }
        if parts.is_empty() {
            return Err(AppError::Extraction(
                "ddg search returned no results (try setting a Serper API key)".into(),
            ));
        }
        Ok(parts.join("\n\n"))
    }
}

/// True if the URL points to a YouTube video.
fn is_youtube(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("youtube.com/watch")
        || lower.contains("youtu.be/")
        || lower.contains("youtube.com/shorts/")
        || lower.contains("youtube.com/embed/")
}

fn youtube_video_id(url: &str) -> Option<String> {
    let url = url.trim();
    if let Some(idx) = url.find("v=") {
        let rest = &url[idx + 2..];
        let id: String = rest.chars().take(11).collect();
        return if id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            Some(id)
        } else {
            None
        };
    }
    for prefix in ["youtu.be/", "youtube.com/shorts/", "youtube.com/embed/"] {
        if let Some(idx) = url.find(prefix) {
            let rest = &url[idx + prefix.len()..];
            let id: String = rest
                .chars()
                .take(11)
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if id.len() == 11 {
                return Some(id);
            }
        }
    }
    None
}

/// Pull the `ytInitialPlayerResponse = {...};` JSON out of a watch page.
fn extract_yt_initial(page: &str) -> Option<Value> {
    let marker = "ytInitialPlayerResponse";
    let idx = page.find(marker)?;
    let rest = &page[idx + marker.len()..];
    let start = rest.find('{')?;
    // Brace-match to find the end of the JSON object.
    let bytes = rest.as_bytes()[start..].iter();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut end = None;
    for (i, b) in bytes.enumerate() {
        match *b {
            b'"' => {
                if !escape {
                    in_str = !in_str;
                }
                escape = false;
            }
            b'\\' => escape = true,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    serde_json::from_str(&rest[start..end]).ok()
}

/// Minimal timedtext XML parsing: `<text start=... d=...>content</text>`.
fn parse_timedtext_xml(xml: &str) -> String {
    let mut out = String::new();
    let mut pos = 0;
    while let Some(start) = xml[pos..].find("<text ") {
        let tag_start = pos + start;
        let tag_end = match xml[tag_start..].find('>') {
            Some(i) => tag_start + i + 1,
            None => break,
        };
        let close = match xml[tag_end..].find("</text>") {
            Some(i) => tag_end + i,
            None => break,
        };
        let inner = &xml[tag_end..close];
        let cleaned = decode_xml_entities(&clean_text(inner));
        if !cleaned.trim().is_empty() {
            out.push_str(&cleaned);
            out.push(' ');
        }
        pos = close + 7;
    }
    out
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
}

/// Collapse whitespace, drop long runs of blank lines.
fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let line: String = line.split_whitespace().collect();
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
