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
    /// "openai" | "elevenlabs" | "edge" | "gemini"
    pub default_tts_model: String,
    pub openai: VoiceConfig,
    pub elevenlabs: VoiceConfig,
    pub edge: VoiceConfig,
    pub gemini: VoiceConfig,
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
