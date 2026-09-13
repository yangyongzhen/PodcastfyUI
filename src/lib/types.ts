// TypeScript mirrors of the Rust backend types (src-tauri/src/...).

export type TaskStatus =
  | "pending"
  | "extracting"
  | "generating"
  | "synthesizing"
  | "muxing"
  | "exporting"
  | "completed"
  | "failed"
  | "cancelled";

export interface TaskInput {
  urls: string[];
  pdfs: string[];
  text: string;
  topic: string;
  longform: boolean;
}

export interface Task {
  id: string;
  title: string;
  input: TaskInput;
  status: TaskStatus;
  progress: number;
  stage: string;
  error: string | null;
  created_at: string;
  transcript_path: string | null;
  audio_path: string | null;
  /** 已导出的视频（横版 video.mp4 / 竖版 video-portrait.mp4）。 */
  video_path: string | null;
  /** 视频导出失败原因；与 error 分开——音频产物仍然有效。 */
  video_error: string | null;
}

export interface ApiKeys {
  openai: string;
  anthropic: string;
  gemini: string;
  elevenlabs: string;
  serper: string;
  /** Exa 托管 MCP：可空——不填也能搜，填了提高配额。 */
  exa: string;
  /** 博查 Web Search（国内）。 */
  bocha: string;
  /** 智谱 Web Search（国内）。 */
  zhipu: string;
  /** 百度千帆 AI 搜索（国内）。 */
  qianfan: string;
  doubao_app_id: string;
  doubao_access_token: string;
}

export interface LlmConfig {
  provider: string;
  model: string;
  base_url: string;
  temperature: number;
  max_tokens: number;
}

export interface VideoConfig {
  /** "landscape"（16:9）| "portrait"（9:16）—— 一次只出一种画幅。 */
  aspect: string;
  /** "wave"（波形/频谱，L2）| "cover"（静态封面，L1）。 */
  style: string;
  /** "generated"（应用自绘底图）| "custom"（用 cover_path）。 */
  cover: string;
  cover_path: string;
  /** 画面主标题，空 = 用任务标题。 */
  title: string;
  /** 画面副标题，空 = 不画。 */
  subtitle: string;
  /** 自定义中文字体路径，空 = 用随包字体。 */
  font_path: string;
}

/** 主题搜索配置（search.json）：后端、条数、全部失败时是否降级。 */
export interface SearchConfig {
  /** "auto" | "exa" | "serper" | "bocha" | "zhipu" | "qianfan" | "ddg" */
  provider: string;
  /** 结果条数 1-20。 */
  num_results: number;
  /** 所有后端都失败时，用模型自带知识继续生成（转录稿会标注未经联网检索）。 */
  degrade_without_search: boolean;
}

export interface VoiceConfig {
  question: string;
  answer: string;
  model: string | null;
}

export interface TtsConfig {
  default_tts_model: string;
  openai: VoiceConfig;
  elevenlabs: VoiceConfig;
  edge: VoiceConfig;
  doubao: VoiceConfig;
  gemini: VoiceConfig;
  audio_format: string;
  ending_message: string;
}

export interface ConversationConfig {
  conversation_style: string[];
  roles_person1: string;
  roles_person2: string;
  dialogue_structure: string[];
  podcast_name: string;
  podcast_tagline: string;
  output_language: string;
  engagement_techniques: string[];
  creativity: number;
  user_instructions: string;
  max_num_chunks: number;
  min_chunk_size: number;
  text_to_speech: TtsConfig;
}
