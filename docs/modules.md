# 模块说明（Rust 后端）

> 路径根：`src-tauri/src/`。各模块职责、公共 API、依赖关系如下。

## 依赖关系

```
lib.rs ──→ queue ──→ pipeline ──→ extractor
              │            ├──→ generator
              │            ├──→ tts
              │            └──→ audio
              └────────────→ config
error.rs（被所有模块使用）
```

## config/mod.rs

| 项 | 说明 |
|---|---|
| `ApiKeys` | openai / anthropic / gemini / elevenlabs / serper 五个字段，空串 = 未配置 |
| `LlmConfig` | provider / model / base_url / temperature / max_tokens |
| `ConversationConfig` | 播客人设（风格、双角色、结构、语言、互动技巧、创造力、长片参数）+ `TtsConfig`（默认 TTS、各 provider 的 question/answer 音色） |
| `load_settings(app)` | 一次性从 app_data_dir 读三份配置（crate 内部用，pipeline 调用） |
| 存储位置 | `app_data_dir/api_keys.json`（0600）、`llm.json`、`conversation.yaml` |

命令：`get_api_keys / save_api_keys / get_llm_config / save_llm_config / get_conversation_config / save_conversation_config`。
所有配置缺失时返回 `Default`（与上游默认值一致），保证首次启动零配置可用（edge TTS + openai LLM 除外需 key）。

## extractor/mod.rs

`Extractor`（持有 `reqwest::Client`，60s 超时，浏览器 UA）：

| 方法 | 输入 | 输出 | 备注 |
|---|---|---|---|
| `extract_url` | 任意 URL | 带 `Title/URL` 头的文本 | 自动分流 YouTube / 网页 |
| `extract_webpage` | http(s) URL | 正文文本 | Readability 打分取主内容；<200 字符时 fallback 到 `<article>/<main>` 选择器；>60k 字符截断 |
| `extract_youtube` | youtube 链接 | 字幕全文 | watch 页 → 括号匹配 `ytInitialPlayerResponse` JSON → 取 captionTracks（优先 en）→ timedtext XML 解析 |
| `extract_pdf` | 本地路径 | 纯文本 | `spawn_blocking` 包 `pdf-extract::extract_text`；扫描版 PDF 报错提示 |
| `search_topic` | 主题 + 可选 Serper key | 5 条搜索结果（标题/摘要/链接） | 无 key 走 DuckDuckGo HTML 版 |

关键辅助：`youtube_video_id`（支持 watch / youtu.be / shorts / embed）、`parse_timedtext_xml`、`clean_text`（按行压缩空白）。

## generator/mod.rs

- `Provider`：`OpenAi | Anthropic | Gemini | Ollama`（`Copy`），`parse()` 宽松匹配。
- `Generator::new(GeneratorConfig)` + `generate_qa(content, conv, longform, out, progress)`。
- 四 provider 的请求细节：
  - **OpenAI 兼容**：`{base}/chat/completions`，默认 `https://api.openai.com/v1`；
  - **Anthropic**：`/v1/messages`，system 字段 + `anthropic-version: 2023-06-01`；
  - **Gemini**：v1beta `generateContent`，key 走 query param，systemInstruction 可选注入；
  - **Ollama**：走 OpenAI 兼容路径，默认 `http://localhost:11434/v1`。
- 长片策略：`split_chunks(content, max_num_chunks, min_chunk_size)` 按换行边界切块，
  每轮把上一轮尾部 4000 字作为上下文；prompt 明确要求「不要收尾」。
- prompt 结构（对齐上游）：系统提示 = 播客名/双角色/风格/结构/语言；
  用户提示 = 风格参数 + 自定义指令 + 长片上下文 + 源材料（截断 60k 字符）。
- 输出格式约定：`PERSON_1: …` / `PERSON_2: …` 逐行，外加播客头尾（`format_transcript`）。
- 超时 600s（长生成场景）。

## tts/mod.rs

- `TtsProvider` trait：`name()` / `synthesize_line(text, voice, out_path)`（async_trait）。
- `create_provider(model, keys, conv)` 工厂：
  - `openai` → `OpenAiTts`（`/v1/audio/speech`，tts-1-hd，mp3，300s 超时）；
  - `edge` → `EdgeTts`（免 key，Bing 端点 + base64 JSON config + SSML 请求体，
    输出 `audio-24khz-96kbitrate-mono-mp3`）；
  - 其它 → 明确报错「本版本不支持」。
- **已知风险**：Edge 走 HTTP JSON 协议（上游 Python 库同款思路），微软改协议即失效，
  错误信息会提示切换到 OpenAI。

## audio/mod.rs

- `find_ffmpeg()`：可执行文件同目录 → PATH，找不到报安装提示。
- `concatenate_parts(parts, out)`：ffmpeg concat demuxer（列表文件写 `out.concat.txt`，
  完成后清理），统一输出 24kHz / mono / 96k mp3；单段时直接 reencode。
- `reencode(src, out)`：同参数重编码，保证格式一致。
- 全部 `spawn_blocking` 包 `std::process::Command`，stderr 尾部 800 字符进错误信息。

## queue/mod.rs + queue/pipeline.rs

- `TaskStatus` 状态机：`pending → extracting → generating → synthesizing → muxing → completed`，
  任意阶段可 `failed / cancelled`。
- `QueueManager`：`Arc<Mutex<HashMap<id, Task>>>`，`Clone` 只复制 Arc（闭包捕获安全）；
  `update()` 内修改 + `emit("task-update")`。
- 命令：`start_task / list_tasks / get_task / cancel_task / delete_task /
  get_transcript / save_transcript / open_audio_file`。
- `pipeline::run` 四阶段与进度映射：
  - 抽取 0→15%（URL 4 并发批处理，`AtomicUsize` 计数）
  - 生成 15→50%（generator 的 progress 回调重映射）
  - TTS 50→90%（`split_transcript` 按 PERSON_1/2 切句，每 3 句一批 join_all）
  - 拼接 90→100%
- 产物：`app_data_dir/tasks/{id}/{transcript.txt, podcast.mp3, parts/*.mp3}`。

## 前端（src/）

- `lib/types.ts`：与 Rust 结构一一对应的 TS 类型（手工镜像，改 Rust 结构时同步）。
- `lib/api.ts`：全部 `invoke` 封装 + `onTaskUpdate` 事件订阅 + `fileToAssetUrl`。
- `routes/+page.svelte`：三视图切换（新建/任务/设置），事件驱动 refresh + 5s 轮询兜底。
- `routes/NewTask.svelte`：输入表单（URL 多行 / PDF 多行 / 文本 / 主题 / 长片开关）。
- `routes/TaskCard.svelte`：进度条 + 阶段文案 + 音频播放 + 转录稿查看/编辑/保存。
- `routes/Settings.svelte`：API keys / LLM / 人设 / 音色，整体保存。
