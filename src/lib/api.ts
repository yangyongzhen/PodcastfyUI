// Thin wrappers around the Tauri commands + event subscription.

import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ApiKeys,
  ConversationConfig,
  LlmConfig,
  OutputConfig,
  SearchConfig,
  Task,
  TaskInput,
  VideoConfig,
} from "./types";

export async function startTask(
  title: string,
  input: TaskInput,
): Promise<string> {
  return invoke("start_task", { title, input });
}

export async function listTasks(): Promise<Task[]> {
  return invoke("list_tasks");
}

/** 复用已有转录稿，只重跑 TTS + 音频拼接。 */
export async function resynthesizeTask(id: string): Promise<void> {
  return invoke("resynthesize_task", { id });
}

export async function cancelTask(id: string): Promise<boolean> {
  return invoke("cancel_task", { id });
}

export async function deleteTask(id: string): Promise<boolean> {
  return invoke("delete_task", { id });
}

export async function getTranscript(id: string): Promise<string | null> {
  return invoke("get_transcript", { id });
}

export async function saveTranscript(id: string, content: string): Promise<void> {
  return invoke("save_transcript", { id, content });
}

/** 打开音频；返回绝对路径（无桌面环境时 xdg-open 会静默失败，前端需自己兜底展示）。 */
export async function openAudioFile(id: string): Promise<string> {
  return invoke("open_audio_file", { id });
}

export async function getApiKeys(): Promise<ApiKeys> {
  return invoke("get_api_keys");
}

export async function saveApiKeys(keys: ApiKeys): Promise<void> {
  return invoke("save_api_keys", { keys });
}

export async function getLlmConfig(): Promise<LlmConfig> {
  return invoke("get_llm_config");
}

export async function saveLlmConfig(config: LlmConfig): Promise<void> {
  return invoke("save_llm_config", { config });
}

export async function getConversationConfig(): Promise<ConversationConfig> {
  return invoke("get_conversation_config");
}

export async function saveConversationConfig(
  config: ConversationConfig,
): Promise<void> {
  return invoke("save_conversation_config", { config });
}

export async function getVideoConfig(): Promise<VideoConfig> {
  return invoke("get_video_config");
}

export async function saveVideoConfig(config: VideoConfig): Promise<void> {
  return invoke("save_video_config", { config });
}

export async function getSearchConfig(): Promise<SearchConfig> {
  return invoke("get_search_config");
}

export async function saveSearchConfig(config: SearchConfig): Promise<void> {
  return invoke("save_search_config", { config });
}

export async function getOutputConfig(): Promise<OutputConfig> {
  return invoke("get_output_config");
}

export async function saveOutputConfig(config: OutputConfig): Promise<void> {
  return invoke("save_output_config", { config });
}

export async function getDefaultOutputDir(): Promise<string> {
  return invoke("get_default_output_dir");
}

/** 导出视频（可选第五阶段）。失败时后端仍保住音频产物，错误只在 video_error 上。 */
export async function exportVideo(id: string): Promise<Task> {
  return invoke("export_video_task", { id });
}

/** 打开视频；返回绝对路径，理由同 openAudioFile。 */
export async function openVideoFile(id: string): Promise<string> {
  return invoke("open_video_file", { id });
}

/** 解析实际会用到的中文字体绝对路径（空串 = 随包字体）。 */
export async function videoFontStatus(fontPath: string): Promise<string> {
  return invoke("video_font_status", { fontPath });
}

/** 连通性探针：kind = "llm" | "tts" | "ffmpeg"，走真实生产请求路径。 */
export async function testConnection(
  kind: "llm" | "tts" | "ffmpeg",
): Promise<string> {
  return invoke("test_connection", { kind });
}

/** Subscribe to live task updates. Returns an unlisten function. */
export function onTaskUpdate(
  cb: (task: Task) => void,
): Promise<UnlistenFn> {
  return listen<Task>("task-update", (e) => cb(e.payload));
}

/**
 * Turn a local file path into a playable asset URL.
 * 路径异常时返回空串：`convertFileSrc` 会抛错，而调用点在 `$derived` 里，
 * 一旦抛出去整个页面都会被带走（白屏）。
 */
export function fileToAssetUrl(path: string): string {
  if (!path || typeof path !== "string") return "";
  try {
    return convertFileSrc(path);
  } catch (e) {
    console.error("convertFileSrc failed", path, e);
    return "";
  }
}
