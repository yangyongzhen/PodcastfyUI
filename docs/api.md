# Tauri 命令 API 参考

前端通过 `invoke("<command>", { ... })` 调用，参数名与 Rust 函数参数一致（snake_case）。
封装见 `src/lib/api.ts`。

## 配置类

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `get_api_keys` | — | `ApiKeys` | 读取各家 API key |
| `save_api_keys` | `keys: ApiKeys` | `()` | 保存（unix 下 0600） |
| `get_llm_config` | — | `LlmConfig` | LLM 设置 |
| `save_llm_config` | `config: LlmConfig` | `()` | 保存 LLM 设置 |
| `get_conversation_config` | — | `ConversationConfig` | 播客人设 + TTS 音色 |
| `save_conversation_config` | `config: ConversationConfig` | `()` | 保存会话设置（YAML） |

### 数据结构

```ts
ApiKeys { openai, anthropic, gemini, elevenlabs, serper: string }

LlmConfig {
  provider: "openai" | "anthropic" | "gemini" | "ollama",
  model: string,          // 如 gpt-4o-mini / qwen2.5 / gemini-2.0-flash
  base_url: string,       // 空 = 默认；Ollama 默认 http://localhost:11434/v1
  temperature: number,    // 0–2
  max_tokens: number
}

ConversationConfig {
  conversation_style: string[],
  roles_person1: string, roles_person2: string,
  dialogue_structure: string[],
  podcast_name: string, podcast_tagline: string,
  output_language: string,
  engagement_techniques: string[],
  creativity: number, user_instructions: string,
  max_num_chunks: number, min_chunk_size: number,
  text_to_speech: {
    default_tts_model: "openai" | "edge",
    openai/elevenlabs/edge/gemini: { question, answer, model? },
    audio_format: "mp3", ending_message: string
  }
}
```

## 任务类

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `start_task` | `title: string, input: TaskInput` | `task_id: string` | 创建并异步启动任务 |
| `list_tasks` | — | `Task[]` | 按创建时间倒序 |
| `get_task` | `id` | `Task` | 单个任务 |
| `cancel_task` | `id` | `bool` | 运行中任务标记取消（不中断已发请求） |
| `delete_task` | `id` | `bool` | 从列表移除（磁盘产物保留） |
| `get_transcript` | `id` | `string \| null` | 转录稿全文 |
| `save_transcript` | `id, content` | `()` | 覆盖保存转录稿 |
| `open_audio_file` | `id` | `()` | 用系统默认播放器打开 mp3 |

### 数据结构

```ts
TaskInput {
  urls: string[],     // 网页 / YouTube 链接
  pdfs: string[],     // 本地 PDF 绝对路径
  text: string,       // 纯文本
  topic: string,      // 主题（联网搜索扩展）
  longform: boolean   // 长篇模式
}

Task {
  id, title: string,
  input: TaskInput,
  status: "pending" | "extracting" | "generating" |
          "synthesizing" | "muxing" | "completed" | "failed" | "cancelled",
  progress: number,   // 0–100
  stage: string,      // 人类可读的阶段描述
  error: string | null,
  created_at: string,
  transcript_path: string | null,
  audio_path: string | null
}
```

## 事件

| 事件 | payload | 触发时机 |
|---|---|---|
| `task-update` | `Task` | 任务创建、状态/进度变化、完成/失败 |

前端订阅示例（`src/lib/api.ts`）：

```ts
import { listen } from "@tauri-apps/api/event";
const un = await listen<Task>("task-update", (e) => console.log(e.payload));
```

## 音频播放

本地文件用 `convertFileSrc(path)` 转成 `asset://` URL 后直接给 `<audio src>`（Tauri 内置 asset 协议，无需额外权限配置）。
