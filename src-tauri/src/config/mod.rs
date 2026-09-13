//! Configuration: API keys + conversation settings.
//!
//! API keys are stored as JSON in the app data directory (later: OS keyring).
//! Conversation settings mirror the upstream `conversation_config.yaml`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn app_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// Everything the pipeline needs, loaded from disk.
pub(crate) struct Settings {
    pub keys: ApiKeys,
    pub llm: LlmConfig,
    pub conversation: ConversationConfig,
    pub video: VideoConfig,
    pub search: SearchConfig,
}

pub(crate) fn load_settings(app: &AppHandle) -> Settings {
    let dir = app.path().app_data_dir();
    Settings {
        keys: dir.as_ref().ok().and_then(|d| read_json(&d.join("api_keys.json"))).unwrap_or_default(),
        llm: dir.as_ref().ok().and_then(|d| read_json(&d.join("llm.json"))).unwrap_or_default(),
        conversation: dir
            .as_ref()
            .ok()
            .and_then(|d| {
                let s = std::fs::read_to_string(d.join("conversation.yaml")).ok()?;
                serde_yaml::from_str(&s).ok()
            })
            .unwrap_or_default(),
        video: dir
            .as_ref()
            .ok()
            .and_then(|d| read_json(&d.join("video.json")))
            .unwrap_or_default(),
        // 读时归一化：手工改过的 search.json 也不能让管道拿到非法 provider。
        search: dir
            .as_ref()
            .ok()
            .and_then(|d| read_json::<SearchConfig>(&d.join("search.json")))
            .map(|mut c| {
                c.normalize();
                c
            })
            .unwrap_or_default(),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(p: &std::path::Path) -> Option<T> {
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn keys_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_dir(app)?.join("api_keys.json"))
}

fn conv_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_dir(app)?.join("conversation.yaml"))
}

/// LLM/TTS provider API keys (empty string = not configured).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ApiKeys {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
    pub elevenlabs: String,
    pub serper: String,
    /// Exa（托管 MCP `https://mcp.exa.ai/mcp`）：可空——无 key 也能搜，填了提高配额。
    pub exa: String,
    /// 博查 Web Search（api.bochaai.com）：国内后端，需 key。
    pub bocha: String,
    /// 智谱 Web Search（open.bigmodel.cn）：国内后端，需 key。
    pub zhipu: String,
    /// 百度千帆 AI 搜索（qianfan.baidubce.com）：国内后端，需 key。
    pub qianfan: String,
    /// 豆包（火山引擎）TTS：应用 App ID（为空表示未配置）。
    pub doubao_app_id: String,
    /// 豆包（火山引擎）TTS：Access Token（为空表示未配置）。
    pub doubao_access_token: String,
    /// 豆包（火山引擎）TTS：资源 ID（X-Api-Resource-Id）。大模型 TTS（bigtts 音色）用
    /// `volc.service_type.10029`；为空时回退到该默认值。
    pub doubao_resource_id: String,
    /// 豆包（火山引擎）TTS：协议版本 `"v1"`（ws_binary）| `"v3"`（bidirection）；为空时回退 `v1`。
    pub doubao_api_version: String,
}

/// Transcript (LLM) generation settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// "openai" | "anthropic" | "gemini" | "ollama"
    pub provider: String,
    pub model: String,
    /// Custom base URL (OpenAI-compatible server / Ollama / Gemini endpoint).
    pub base_url: String,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
            base_url: String::new(),
            temperature: 0.8,
            max_tokens: 4096,
        }
    }
}

/// 视频导出设置（L1 静态封面 / L2 波形）。
///
/// 独立于播客人设（那属于内容），这里只描述「怎么出片」，存 `video.json`。
/// 两项字符串枚举式字段都走 [`VideoConfig::normalize`] 归一化，非法值回落到默认，
/// 保证前端传入任何值都不会让导出阶段崩掉。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    /// 画幅：`"landscape"`（16:9，1280×720）| `"portrait"`（9:16，720×1280）。
    /// 一次只出一种画幅，由本字段决定，不做两套并出。
    pub aspect: String,
    /// 视觉风格：`"wave"`（波形/频谱，L2）| `"cover"`（静态封面，L1）。
    pub style: String,
    /// 封面来源：`"generated"`（应用自绘底图）| `"custom"`（用 `cover_path`）。
    pub cover: String,
    /// 自定义封面图路径（仅 `cover = "custom"` 时生效）。
    pub cover_path: String,
    /// 画面主标题；为空则用任务标题。
    pub title: String,
    /// 画面副标题（如播客 tagline）；为空则不画。
    pub subtitle: String,
    /// 自定义中文字体文件路径；为空则用随包字体（见 `src-tauri/fonts/`）。
    pub font_path: String,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            aspect: "landscape".into(),
            style: "wave".into(),
            cover: "generated".into(),
            cover_path: String::new(),
            title: String::new(),
            subtitle: String::new(),
            font_path: String::new(),
        }
    }
}

impl VideoConfig {
    /// 归一化枚举式字段：容忍大小写、中英文别名，其余一律回落默认值。
    pub fn normalize(&mut self) {
        self.aspect = match self.aspect.trim().to_lowercase().as_str() {
            "portrait" | "vertical" | "9:16" | "竖版" | "竖屏" => "portrait".into(),
            _ => "landscape".into(),
        };
        self.style = match self.style.trim().to_lowercase().as_str() {
            "cover" | "static" | "封面" | "静态封面" => "cover".into(),
            _ => "wave".into(),
        };
        self.cover = match self.cover.trim().to_lowercase().as_str() {
            "custom" | "file" | "自选" | "自选图" => "custom".into(),
            _ => "generated".into(),
        };
        // 注意：不因 cover != custom 而清空 cover_path——用户来回切换来源时不该丢路径。
    }

    /// 画幅像素尺寸：横版 1280×720，竖版 720×1280。
    pub fn size(&self) -> (u32, u32) {
        if self.aspect == "portrait" {
            (720, 1280)
        } else {
            (1280, 720)
        }
    }

    /// 是否需要用户提供封面图（自选来源但没给图 = 需要前端提示）。
    pub fn needs_cover_file(&self) -> bool {
        self.cover == "custom" && self.cover_path.trim().is_empty()
    }
}

/// 主题搜索（`search.json`）：用哪个后端、取几条、全部失败时怎么办。
///
/// 凭证不在这里，仍放在 `api_keys.json`（见 [`ApiKeys`]）。默认 `provider = "auto"`，
/// 管道按「Exa 托管 MCP → 有 key 的商业后端 → DuckDuckGo」依次尝试。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// `"auto"` | `"exa"` | `"serper"` | `"bocha"` | `"zhipu"` | `"qianfan"` | `"ddg"`。
    pub provider: String,
    /// 期望结果条数（1-20，默认 5）。
    pub num_results: usize,
    /// 所有后端都失败时，是否降级为「用模型自带知识继续生成」（转录稿会标注未经联网检索）。
    pub degrade_without_search: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            provider: "auto".into(),
            num_results: 5,
            degrade_without_search: true,
        }
    }
}

impl SearchConfig {
    /// 归一化：未知 provider 回落 `auto`，条数夹到 1-20。
    pub fn normalize(&mut self) {
        self.provider = match self.provider.trim().to_lowercase().as_str() {
            "exa" | "exa-mcp" | "mcp" => "exa".into(),
            "serper" | "google" | "serper.dev" => "serper".into(),
            "bocha" | "博查" => "bocha".into(),
            "zhipu" | "glm" | "智谱" => "zhipu".into(),
            "qianfan" | "baidu" | "千帆" | "百度" => "qianfan".into(),
            "ddg" | "duckduckgo" => "ddg".into(),
            _ => "auto".into(),
        };
        self.num_results = self.num_results.clamp(1, 20);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VoiceConfig {
    pub question: String,
    pub answer: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    /// "openai" | "elevenlabs" | "edge" | "gemini" | "doubao"
    pub default_tts_model: String,
    pub openai: VoiceConfig,
    pub elevenlabs: VoiceConfig,
    pub edge: VoiceConfig,
    pub gemini: VoiceConfig,
    pub doubao: VoiceConfig,
    pub audio_format: String,
    pub ending_message: String,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            default_tts_model: "openai".into(),
            openai: VoiceConfig {
                question: "echo".into(),
                answer: "shimmer".into(),
                model: Some("tts-1-hd".into()),
            },
            elevenlabs: VoiceConfig {
                question: "Chris".into(),
                answer: "Jessica".into(),
                model: Some("eleven_multilingual_v2".into()),
            },
            edge: VoiceConfig {
                question: "en-US-JennyNeural".into(),
                answer: "en-US-EricNeural".into(),
                model: None,
            },
            gemini: VoiceConfig {
                question: "en-US-Journey-D".into(),
                answer: "en-US-Journey-O".into(),
                model: None,
            },
            doubao: VoiceConfig {
                // 默认音色（火山引擎 voice_type）。question / answer 暂用同一音色，
                // 用户应在设置中替换为自己所需的男声 / 女声音色。
                question: "zh_female_wanwanxiaohe_moon_bigtts".into(),
                answer: "zh_female_wanwanxiaohe_moon_bigtts".into(),
                // 豆包 TTS 用 model 字段承载 cluster（集群名）。
                model: Some("volcano_tts".into()),
            },
            audio_format: "mp3".into(),
            ending_message: "See You Next Time!".into(),
        }
    }
}

/// Podcast "persona" settings, mirroring upstream conversation_config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversationConfig {
    pub conversation_style: Vec<String>,
    pub roles_person1: String,
    pub roles_person2: String,
    pub dialogue_structure: Vec<String>,
    pub podcast_name: String,
    pub podcast_tagline: String,
    pub output_language: String,
    pub engagement_techniques: Vec<String>,
    pub creativity: f32,
    pub user_instructions: String,
    /// Longform: max rounds of discussion.
    pub max_num_chunks: usize,
    /// Longform: min characters per discussion round.
    pub min_chunk_size: usize,
    pub text_to_speech: TtsConfig,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            conversation_style: vec![
                "engaging".into(),
                "fast-paced".into(),
                "enthusiastic".into(),
            ],
            roles_person1: "main summarizer".into(),
            roles_person2: "questioner/clarifier".into(),
            dialogue_structure: vec![
                "Introduction".into(),
                "Main Content Summary".into(),
                "Conclusion".into(),
            ],
            podcast_name: "PODCASTIFY".into(),
            podcast_tagline: "Your Personal Generative AI Podcast".into(),
            output_language: "English".into(),
            engagement_techniques: vec![
                "rhetorical questions".into(),
                "anecdotes".into(),
                "analogies".into(),
                "humor".into(),
            ],
            creativity: 1.0,
            user_instructions: String::new(),
            max_num_chunks: 8,
            min_chunk_size: 600,
            text_to_speech: TtsConfig::default(),
        }
    }
}

/// 把用户输入的「输出语言」归一为明确的英文表述，供提示词、TTS、字幕轨道选择共用。
///
/// 目的是消除歧义（例如「中文」会被模型理解为繁体），并让下游按统一字符串判断语言。
/// 非中文输入（如 `English`、`Japanese`）原样返回，行为保持不变。
pub fn normalize_language(input: &str) -> String {
    let trimmed = input.trim();
    match trimmed.to_lowercase().as_str() {
        "中文" | "简体" | "简体中文" | "zh" | "zh-cn" | "zh_cn" | "chinese" | "simplified chinese" => {
            "Simplified Chinese (简体中文)".to_string()
        }
        "繁体" | "繁体中文" | "繁體" | "繁體中文" | "zh-tw" | "zh_tw" | "zh-hk" | "traditional chinese" => {
            "Traditional Chinese (繁體中文)".to_string()
        }
        "english" | "en" | "en-us" | "en_us" => "English".to_string(),
        _ => trimmed.to_string(),
    }
}

#[tauri::command]
pub fn get_api_keys(app: AppHandle) -> Result<ApiKeys, String> {
    let p = keys_path(&app)?;
    if !p.exists() {
        return Ok(ApiKeys::default());
    }
    let s = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_api_keys(app: AppHandle, keys: ApiKeys) -> Result<(), String> {
    let p = keys_path(&app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(&keys).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())?;
    // Best-effort: restrict permissions on unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn llm_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_dir(app)?.join("llm.json"))
}

#[tauri::command]
pub fn get_llm_config(app: AppHandle) -> Result<LlmConfig, String> {
    let p = llm_path(&app)?;
    if !p.exists() {
        return Ok(LlmConfig::default());
    }
    let s = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_llm_config(app: AppHandle, config: LlmConfig) -> Result<(), String> {
    let p = llm_path(&app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_conversation_config(app: AppHandle) -> Result<ConversationConfig, String> {
    let p = conv_path(&app)?;
    if !p.exists() {
        return Ok(ConversationConfig::default());
    }
    let s = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    serde_yaml::from_str(&s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_conversation_config(app: AppHandle, config: ConversationConfig) -> Result<(), String> {
    let p = conv_path(&app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_yaml::to_string(&config).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

fn video_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_dir(app)?.join("video.json"))
}

#[tauri::command]
pub fn get_video_config(app: AppHandle) -> Result<VideoConfig, String> {
    let p = video_path(&app)?;
    let mut cfg: VideoConfig = if !p.exists() {
        VideoConfig::default()
    } else {
        let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())?
    };
    // 读时归一化：老配置或被手工改过的文件也保证枚举字段合法。
    cfg.normalize();
    Ok(cfg)
}

#[tauri::command]
pub fn save_video_config(app: AppHandle, mut config: VideoConfig) -> Result<(), String> {
    config.normalize();
    let p = video_path(&app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

fn search_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_dir(app)?.join("search.json"))
}

#[tauri::command]
pub fn get_search_config(app: AppHandle) -> Result<SearchConfig, String> {
    let p = search_path(&app)?;
    let mut cfg: SearchConfig = if !p.exists() {
        SearchConfig::default()
    } else {
        let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())?
    };
    cfg.normalize();
    Ok(cfg)
}

#[tauri::command]
pub fn save_search_config(app: AppHandle, mut config: SearchConfig) -> Result<(), String> {
    config.normalize();
    let p = search_path(&app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}
