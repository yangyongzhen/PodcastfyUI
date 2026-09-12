// Thin wrappers around the Tauri commands + event subscription.

import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ApiKeys,
  ConversationConfig,
  LlmConfig,
  Task,
  TaskInput,
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

export async function openAudioFile(id: string): Promise<void> {
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

/** Turn a local file path into a playable asset URL. */
export function fileToAssetUrl(path: string): string {
  return convertFileSrc(path);
}
