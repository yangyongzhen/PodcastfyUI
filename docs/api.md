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
| `get_video_config` | — | `VideoConfig` | 视频导出设置（读取时归一化非法枚举值） |
| `save_video_config` | `config: VideoConfig` | `()` | 保存视频导出设置（JSON） |
| `video_font_status` | `font_path: string` | `string` | 解析实际使用的中文字体绝对路径（空 = 随包字体） |

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
| `export_video_task` | `id` | `Task` | 把已完成的 mp3 导出成视频（可选第五阶段，见下） |
| `open_video_file` | `id` | `()` | 用系统默认播放器打开已导出的 mp4 |

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
          "synthesizing" | "muxing" | "exporting" |
          "completed" | "failed" | "cancelled",
  progress: number,   // 0–100
  stage: string,      // 人类可读的阶段描述
  error: string | null,
  created_at: string,
  transcript_path: string | null,
  audio_path: string | null,
  video_path: string | null,    // 已导出的 mp4（横版 video.mp4 / 竖版 video-portrait.mp4）
  video_error: string | null    // 视频导出失败原因；与 error 分开，不影响已成功的音频
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

## 主题搜索（search.json + 兜底链）

「只给一个主题」时，管道按下列顺序找素材（`provider = "auto"`）：`exa`（托管 MCP，**免密钥**）
→ `serper` → `bocha` → `zhipu` → `qianfan` → `ddg`。没配 key 的商业后端自动跳过；显式指定
`provider` 时只试该后端。

| 命令 / 密钥字段 | 说明 |
|---|---|
| `get_search_config` / `save_search_config` | 读写 `search.json` |
| `ApiKeys.exa` / `.serper` / `.bocha` / `.zhipu` / `.qianfan` | 各后端密钥，都可空；Exa 留空也能搜（填了提高配额） |

`search.json` 字段：

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `provider` | string | `"auto"` | `auto` / `exa` / `serper` / `bocha` / `zhipu` / `qianfan` / `ddg` |
| `num_results` | number | `5` | 期望条数，归一化夹到 1-20 |
| `degrade_without_search` | bool | `true` | 所有后端都失败时，是否用模型自带知识继续生成 |

失败行为：为真 → 任务继续，素材中标注「未联网核实」，阶段文案为
`search unavailable — using model knowledge…`；为假 → 任务按失败处理，错误提示改用 URL /
粘贴文本或补配密钥。

探针：`cd src-tauri && cargo run --example search_probe [-- "主题" 后端名]`（不读应用配置、
所有 key 为空时验证 auto → Exa）。

## 视频导出（L1 封面 / L2 波形）

| 项 | 说明 |
|---|---|
| 触发方式 | 任务卡片上的「导出视频」按钮。**不自动跟跑**——免得每次生成都多付一次编码时间 |
| 产物 | 横版 `video.mp4`（1280×720）· 竖版 `video-portrait.mp4`（720×1280），落在任务目录内 |
| 画幅 | 由 `VideoConfig.aspect` **单选**决定，一次只出一种，不做两版并出 |
| 风格 | `style = "wave"` 波形（`showwaves`，品牌青）｜ `"cover"` 静态封面 |
| 封面 | `cover = "generated"` 自绘渐变底图（品牌深底 → 青）｜ `"custom"` 用 `cover_path`，铺满画幅后居中裁切 |
| 中文文字 | 标题 + 副标题经 `drawtext` 烧进画面；文本走临时 `textfile`，所以中文、引号、冒号、逗号**都不需要转义** |
| 字体 | 三级查找：`font_path` 自定义 → 资源目录 `fonts/DroidSansFallbackFull.ttf` → 仓库内（开发期）。三级都没有时**提前报错**，不会画出一屏方块 |
| 失败处理 | 只写 `video_error`，任务状态回到 `completed`，音频产物与 `audio_path` 不受任何影响 |
| 取消 | 进入编码前检查一次；ffmpeg 编码过程中不中断（与既有口径一致），收尾时不会把已取消的任务改回「完成」 |

配置由 `VideoConfig` 持久化在 `video.json`：

```ts
VideoConfig {
  aspect: "landscape" | "portrait",   // 横版 16:9 | 竖版 9:16（单选）
  style: "wave" | "cover",            // 波形（L2） | 静态封面（L1）
  cover: "generated" | "custom",      // 自绘底图 | 自选图片
  cover_path: string,                 // 自选图片路径（cover = "custom" 时生效）
  title: string,                      // 空 = 用任务标题
  subtitle: string,                   // 空 = 用播客 tagline
  font_path: string                   // 空 = 用随包中文字体
}
```

导出的 mp4 同样用 `convertFileSrc` 在应用内预览播放（与音频一致）。

## 任务表持久化与重启恢复

| 项 | 说明 |
|---|---|
| 索引文件 | `{app_data_dir}/task_index.json`（`{version, tasks[]}`）；与产物目录 `tasks/` 分开，命名不冲突 |
| 落盘时机 | `start` / `update`（含进度推进）/ `cancel` / `delete` / `save_transcript` 之后立即原子写（`.tmp` → `rename`） |
| 失败策略 | 索引读写与解析失败只记 `warn` 日志，绝不影响任务本身；文件不存在照常启动 |
| 恢复来源 | ① 索引；② 扫描 `tasks/*/` 重建历史任务（有 `podcast.mp3` → completed，只有转录稿/视频 → failed） |
| 标题来源 | 可读目录名 `日期-主题-短id` 的中间段；纯 uuid 的旧目录取转录稿首句（截断 42 字） |
| 状态归一 | `pending` / `extracting` / `generating` / `synthesizing` / `muxing` / `exporting` 载入时统一改 failed + `stage = "interrupted"` |
| 空壳目录 | 三个产物都没有的目录不算任务，直接跳过，列表里不会出现空卡片 |
| 删除语义 | `delete_task` 连产物一起删（音频/转录稿/视频/`parts/`），避免重启时被扫描复活 |
| 恢复日志 | 启动打印 `task index: restored N task(s) (M from workspace, K interrupted)`，供无头核对 |

## 输出目录

| 命令 | 说明 |
| --- | --- |
| `get_output_config` | 读取产物目录配置（`output_dir` 为空 = 用系统应用数据目录） |
| `save_output_config` | 保存并即时生效；自定义目录会经 `allow_asset_dir` 放行给 asset 协议 |
| `get_default_output_dir` | 返回系统默认根目录（设置页「恢复默认」用） |

产物根目录下每个任务一个文件夹，名为 `日期-主题-id前8位`（例：`2026-09-12-我的播客-90a5b181`）；任务成功后清理 `parts/` 中间产物，成品音频/视频/封面保留。
