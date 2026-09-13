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

/// 主题搜索的运行期配置（由 `config::Settings` 组装后传入）。
///
/// `provider` 为空或 `"auto"` 时按 [`Extractor::search_topic`] 的默认顺序依次尝试；
/// 显式指定（如 `"bocha"`）则只试该后端。
#[derive(Debug, Clone, Default)]
pub struct SearchCfg {
    pub provider: String,
    pub num_results: usize,
    /// Exa MCP API key（可空：托管端点不带 key 也能用，带 key 提高配额）。
    pub exa_key: String,
    pub serper_key: String,
    pub bocha_key: String,
    pub zhipu_key: String,
    pub qianfan_key: String,
}

/// Exa 托管 MCP（Streamable HTTP）端点。
const EXA_MCP_URL: &str = "https://mcp.exa.ai/mcp";

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
        // Validate before touching the network: a non-URL (e.g. a topic typed
        // into the URL field) otherwise surfaces as reqwest's opaque
        // "builder error".
        let url_parsed = url::Url::parse(url).map_err(|e| {
            AppError::Extraction(format!(
                "'{url}' is not a valid URL ({e}) — put topics in the Topic field, or paste the text directly"
            ))
        })?;
        let html_text = self
            .http
            .get(url_parsed.clone())
            .header("Accept", "text/html,application/xhtml+xml")
            .send()
            .await
            .map_err(|e| AppError::Extraction(format!("fetch {url}: {e}")))?
            .text()
            .await
            .map_err(|e| AppError::Extraction(format!("read body {url}: {e}")))?;

        let parsed = Html::parse_document(&html_text);
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

    /// Expand a topic into background material using the first search backend
    /// that answers.
    ///
    /// `"auto"` tries Exa's hosted MCP endpoint (no API key needed), then every
    /// keyed backend (Serper / BoCha / 智谱 / 千帆), then DuckDuckGo HTML search.
    /// An explicit `cfg.provider` tries that single backend only.
    pub async fn search_topic(&self, topic: &str, cfg: &SearchCfg) -> Result<String, AppError> {
        let provider = cfg.provider.trim().to_lowercase();
        let explicit = !provider.is_empty() && provider != "auto";
        let num = if cfg.num_results == 0 {
            5
        } else {
            cfg.num_results.clamp(1, 20)
        };

        let mut order: Vec<&str> = if explicit {
            vec![provider.as_str()]
        } else {
            vec!["exa", "serper", "bocha", "zhipu", "qianfan", "ddg"]
        };
        // 没配 key 的商业后端直接跳过（exa / ddg 不需要 key）。
        order.retain(|p| match *p {
            "serper" => !cfg.serper_key.trim().is_empty(),
            "bocha" => !cfg.bocha_key.trim().is_empty(),
            "zhipu" => !cfg.zhipu_key.trim().is_empty(),
            "qianfan" => !cfg.qianfan_key.trim().is_empty(),
            _ => true,
        });
        if order.is_empty() {
            return Err(AppError::Extraction(format!(
                "topic search has no usable backend: provider '{provider}' needs an API key in Settings — or use a URL / pasted text instead"
            )));
        }

        let mut last_err: Option<AppError> = None;
        for name in order {
            let attempt = match name {
                "exa" => self.search_topic_exa(topic, cfg.exa_key.trim(), num).await,
                "serper" => self.search_topic_serper(topic, cfg.serper_key.trim(), num).await,
                "bocha" => self.search_topic_bocha(topic, cfg.bocha_key.trim(), num).await,
                "zhipu" => self.search_topic_zhipu(topic, cfg.zhipu_key.trim(), num).await,
                "qianfan" => self.search_topic_qianfan(topic, cfg.qianfan_key.trim(), num).await,
                "ddg" => self.search_topic_ddg(topic).await,
                other => Err(AppError::Extraction(format!("unknown search provider '{other}'"))),
            };
            match attempt {
                Ok(text) => {
                    tracing::info!("topic search [{name}]: ok ({} chars)", text.len());
                    return Ok(text);
                }
                Err(e) => {
                    tracing::warn!("topic search [{name}]: failed: {e}");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| AppError::Extraction("topic search failed".into())))
    }

    /// Exa 托管 MCP（Streamable HTTP）。不带 key 也能搜，返回的是干净正文，
    /// 可直接拼进转录稿提示词；带 key（`?exaApiKey=`）提高配额。
    async fn search_topic_exa(&self, topic: &str, key: &str, num: usize) -> Result<String, AppError> {
        let url = if key.is_empty() {
            EXA_MCP_URL.to_string()
        } else {
            let safe: String = key
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            format!("{EXA_MCP_URL}?exaApiKey={safe}")
        };
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": { "query": topic, "numResults": num }
            }
        });
        let resp = self
            .http
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .json(&body)
            .timeout(Duration::from_secs(25))
            .send()
            .await
            .map_err(|e| {
                AppError::Extraction(format!(
                    "topic search unavailable: Exa MCP request failed ({e}) — check the network, or use a URL / pasted text instead"
                ))
            })?;
        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| AppError::Extraction(format!("exa mcp body: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Extraction(format!(
                "exa mcp error {status}: {}",
                Self::snippet(&raw)
            )));
        }
        let v = Self::mcp_json(&raw)?;
        if let Some(err) = v.get("error") {
            return Err(AppError::Extraction(format!("exa mcp error: {err}")));
        }
        let result = v.get("result").ok_or_else(|| {
            AppError::Extraction(format!("exa mcp: no result in response ({})", Self::snippet(&raw)))
        })?;
        if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            return Err(AppError::Extraction(format!(
                "exa mcp tool error: {}",
                Self::mcp_text(result).unwrap_or_default()
            )));
        }
        let text = Self::mcp_text(result).unwrap_or_default();
        if text.trim().is_empty() {
            return Err(AppError::Extraction("exa mcp returned no results".into()));
        }
        Ok(text)
    }

    /// 博查 Web Search（国内）。文档：<https://open.bochaai.com/>
    /// 结果在 `data.webPages.value[]`（部分版本直接是 `webPages.value[]`）。
    async fn search_topic_bocha(&self, topic: &str, key: &str, num: usize) -> Result<String, AppError> {
        let resp = self
            .http
            .post("https://api.bochaai.com/v1/web-search")
            .header("Authorization", format!("Bearer {key}"))
            .json(&serde_json::json!({
                "query": topic,
                "count": num,
                "summary": true,
                "freshness": "noLimit"
            }))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| {
                AppError::Extraction(format!(
                    "topic search unavailable: BoCha request failed ({e}) — check the network / the BoCha API key, or use a URL / pasted text instead"
                ))
            })?;
        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Extraction(format!("bocha json: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Extraction(format!(
                "bocha error {status}: {}",
                Self::api_err_text(&v)
            )));
        }
        let hits: Vec<(String, String, String)> = v
            .pointer("/data/webPages/value")
            .or_else(|| v.pointer("/webPages/value"))
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        let summary = Self::str_at(r, "summary");
                        let snippet = if summary.is_empty() {
                            Self::str_at(r, "snippet")
                        } else {
                            summary
                        };
                        (Self::str_at(r, "name"), Self::str_at(r, "url"), snippet)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::format_hits(&hits)
            .ok_or_else(|| AppError::Extraction("bocha returned no results".into()))
    }

    /// 智谱 Web Search API（国内）。文档：<https://docs.bigmodel.cn/cn/guide/tools/web-search>
    async fn search_topic_zhipu(&self, topic: &str, key: &str, num: usize) -> Result<String, AppError> {
        let resp = self
            .http
            .post("https://open.bigmodel.cn/api/paas/v4/web_search")
            .header("Authorization", format!("Bearer {key}"))
            .json(&serde_json::json!({
                "search_engine": "search_std",
                "search_query": topic,
                "count": num,
                "content_size": "medium"
            }))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| {
                AppError::Extraction(format!(
                    "topic search unavailable: 智谱 request failed ({e}) — check the network / the 智谱 API key, or use a URL / pasted text instead"
                ))
            })?;
        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Extraction(format!("zhipu json: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Extraction(format!(
                "zhipu error {status}: {}",
                Self::api_err_text(&v)
            )));
        }
        let hits: Vec<(String, String, String)> = v
            .get("search_result")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        (
                            Self::str_at(r, "title"),
                            Self::str_at(r, "link"),
                            Self::str_at(r, "content"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::format_hits(&hits)
            .ok_or_else(|| AppError::Extraction("zhipu returned no results".into()))
    }

    /// 百度千帆 AI 搜索（国内）。
    /// 文档：<https://cloud.baidu.com/doc/qianfan-api/s/Wmbq4z7e5>
    async fn search_topic_qianfan(&self, topic: &str, key: &str, num: usize) -> Result<String, AppError> {
        let resp = self
            .http
            .post("https://qianfan.baidubce.com/v2/ai_search/web_search")
            .header("Authorization", format!("Bearer {key}"))
            .json(&serde_json::json!({
                "messages": [{ "role": "user", "content": topic }],
                "search_source": "baidu_search_v2",
                "resource_type_filter": [{ "type": "web", "top_k": num }]
            }))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| {
                AppError::Extraction(format!(
                    "topic search unavailable: 千帆 request failed ({e}) — check the network / the 千帆 API key, or use a URL / pasted text instead"
                ))
            })?;
        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Extraction(format!("qianfan json: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Extraction(format!(
                "qianfan error {status}: {}",
                Self::api_err_text(&v)
            )));
        }
        let hits: Vec<(String, String, String)> = v
            .get("references")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        (
                            Self::str_at(r, "title"),
                            Self::str_at(r, "url"),
                            Self::str_at(r, "content"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::format_hits(&hits)
            .ok_or_else(|| AppError::Extraction("qianfan returned no results".into()))
    }

    /// MCP 响应体解析：Streamable HTTP 用 SSE（`data: {...}` 行），但也兼容纯 JSON。
    fn mcp_json(body: &str) -> Result<Value, AppError> {
        let mut last: Option<Value> = None;
        for line in body.lines() {
            let Some(rest) = line.strip_prefix("data:") else {
                continue;
            };
            let rest = rest.trim();
            if rest.is_empty() || rest == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(rest) {
                last = Some(v);
            }
        }
        if let Some(v) = last {
            return Ok(v);
        }
        serde_json::from_str(body.trim())
            .map_err(|e| AppError::Extraction(format!("mcp json: {e}")))
    }

    /// 取 MCP `result.content[].text` 拼成纯文本（Exa 把结果放在这里）。
    fn mcp_text(result: &Value) -> Option<String> {
        let parts: Vec<&str> = result
            .get("content")?
            .as_array()?
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }

    /// 取字符串字段（缺失或非字符串 → 空串）。
    fn str_at(v: &Value, key: &str) -> String {
        v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    /// 统一把 `(标题, 链接, 摘要)` 列表格式化成喂给 LLM 的素材文本。
    fn format_hits(hits: &[(String, String, String)]) -> Option<String> {
        if hits.is_empty() {
            return None;
        }
        Some(
            hits.iter()
                .map(|(title, url, snippet)| format!("### {title}\n{snippet}\n({url})"))
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }

    /// 各家错误响应里尽量捞一句人话。
    fn api_err_text(v: &Value) -> String {
        for p in ["/error/message", "/error_msg", "/message", "/msg", "/error"] {
            if let Some(s) = v.pointer(p).and_then(Value::as_str) {
                return s.to_string();
            }
        }
        Self::snippet(&v.to_string())
    }

    /// 错误信息里附带的响应片段（别把整段 HTML/JSON 塞进 UI）。
    fn snippet(s: &str) -> String {
        let t = s.trim();
        if t.chars().count() > 200 {
            format!("{}…", t.chars().take(200).collect::<String>())
        } else {
            t.to_string()
        }
    }

    async fn search_topic_serper(&self, topic: &str, key: &str, num: usize) -> Result<String, AppError> {
        let resp = self
            .http
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", key)
            .json(&serde_json::json!({ "q": topic, "num": num }))
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| {
                AppError::Extraction(format!(
                    "topic search unavailable: Serper request failed ({e}) — check the network / the Serper API key, or use a URL / pasted text instead"
                ))
            })?;
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
        // Short timeout on purpose: when DuckDuckGo is unreachable (blackholed
        // route, no IPv6 default route, ...) the failure must surface in
        // seconds instead of after the client-wide 60s.
        let html = self
            .http
            .post("https://html.duckduckgo.com/html/")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[("q", topic)])
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| {
                AppError::Extraction(format!(
                    "topic search unavailable: DuckDuckGo is unreachable ({e}) — set a Serper API key in Settings, or use a URL / pasted text instead"
                ))
            })?
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
