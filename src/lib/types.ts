// TypeScript mirrors of the Rust backend types (src-tauri/src/...).

export type TaskStatus =
  | "pending"
  | "extracting"
  | "generating"
  | "synthesizing"
  | "muxing"
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
}

export interface ApiKeys {
  openai: string;
  anthropic: string;
  gemini: string;
  elevenlabs: string;
  serper: string;
}

export interface LlmConfig {
  provider: string;
  model: string;
  base_url: string;
  temperature: number;
  max_tokens: number;
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
