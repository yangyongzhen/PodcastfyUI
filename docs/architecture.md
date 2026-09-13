# PodcastfyUI 架构设计

> 用 **Tauri v2 + 纯 Rust** 复刻 [podcastfy](https://github.com/souzatharsis/podcastfy)（NotebookLM 播客功能的开源版），提供图形界面。

## 1. 总体架构

```
┌────────────────────────────────────────────────────────────┐
│  前端 (Svelte 5 + SvelteKit SPA, adapter-static)           │
│  +page.svelte → NewTask / TaskCard / Settings              │
│  src/lib/api.ts  ←→  invoke()  (Tauri IPC)                 │
└──────────────┬─────────────────────────────────────────────┘
               │  commands + events("task-update")
┌──────────────▼─────────────────────────────────────────────┐
│  Rust 后端 (src-tauri/src)                                  │
│  lib.rs        命令注册 / AppState(QueueManager)            │
│  queue/        任务队列 + 状态机 + pipeline.rs(真实管道)     │
│  config/       ApiKeys(JSON) / LlmConfig(JSON) / 会话 YAML  │
│  extractor/    网页(Readability) / YouTube 字幕 / PDF / 搜索 │
│  generator/    LLM 对话稿（OpenAI 兼容/Anthropic/Gemini/Ollama）│
│  tts/          TTS trait + 工厂（OpenAI / Edge 免费）        │
│  audio/        ffmpeg sidecar 拼接                          │
└────────────────────────────────────────────────────────────┘
```

设计原则：

1. **无本地 ML**：整条流水线是「HTTP API 编排 + 文本处理 + ffmpeg」，Rust 用 `reqwest` + tokio 即可完整复刻，无需 Python。
2. **异步长任务**：任务在 tokio task 中运行，通过 Tauri `task-update` 事件向前端推进度；前端另有 5s 轮询兜底（窗口隐藏时事件可能丢失）。
3. **配置分层**（与上游一致，但更安全）：
   - `api_keys.json`（0600 权限）：各家 API key
   - `llm.json`：LLM provider/model/base_url/temperature
   - `conversation.yaml`：播客人设 + TTS 音色（字段对齐上游 `conversation_config.yaml`）
4. **产物目录**：`{app_data_dir}/tasks/{task_id}/` 下放 `transcript.txt`、`podcast.mp3`、`parts/`（逐句音频）。
5. **任务表持久化**：`{app_data_dir}/task_index.json` 保存任务列表（`{version, tasks[]}`），所有变更点写后即存
   （先写 `.tmp` 再 `rename`）；启动时「读索引 + 扫描 `tasks/*/`」两路合一恢复，历史任务也能回到列表里。

## 2. 生成流水线（pipeline.rs）

> 主题搜索（只给主题、不给 URL 时）：`extractor::search_topic()` 按
> **exa（托管 MCP，免密钥）→ serper → bocha → zhipu → qianfan → ddg** 依次尝试，没配 key 的后端
> 自动跳过；全部失败且 `search.json` 的 `degrade_without_search` 为真（默认）时，管道降级为
> 「用模型自带知识继续生成」并在素材里标注未联网核实。详见 `docs/api.md` 的「主题搜索」。

```
输入(urls/pdfs/text/topic/longform)
  │
  ├─ 1. 抽取 0-15%    网页(4 并发) / PDF 串行 / 文本 / 主题搜索(Serper→DDG 兜底)
  │
  ├─ 2. 对话稿 15-50%  短片=单请求；长片=分块(max_num_chunks)滚动多轮，
  │                   上一轮尾部 4000 字作为上下文回喂
  │
  ├─ 3. TTS 50-90%     按 "PERSON_1:/PERSON_2:" 切句，每句按角色选音色，
  │                   3 句一批并发合成 mp3
  │
  └─ 4. 拼接 90-100%   ffmpeg concat demuxer → 24kHz mono mp3
```

失败语义：任一阶段出错 → 任务 `failed` + error 文案（含阶段定位，如 "TTS line 7 failed"），转录稿等中间产物保留。

## 3. 与上游 Python 版的对应关系

| 上游模块 | 本项目 | 差异 |
|---|---|---|
| `content_parser/` | `extractor/` | 功能等价；搜索默认走 Serper，无 key 时用 DDG HTML |
| `content_generator.py` (LangChain) | `generator/` | 手写 HTTP 编排，去掉 LangChain 依赖；prompt 结构对齐（双角色/结构/风格/互动技巧） |
| `tts/base.py` + `factory.py` + `providers/` | `tts/mod.rs` | 同一套 trait+工厂抽象；本期实现 OpenAI + Edge，ElevenLabs/Gemini multi 留接口 |
| pydub + ffmpeg | `audio/` | ffmpeg sidecar 拼接，统一 24kHz mono 输出 |
| CLI (typer) | GUI + 相同参数集 | `TaskInput` 覆盖 `--url/--file/--transcript/--image/--text/--topic/--longform` 的主要能力（image/transcript 输入见 TODO） |

## 4. 已知限制 / 风险

- **Edge TTS 是逆向协议**，微软随时可能改；失败时 UI 会提示改用 OpenAI TTS。
- **YouTube 字幕**依赖 `ytInitialPlayerResponse` 结构，反爬变化时降级为手动粘贴文本。
- API keys 目前存本地 JSON（0600），后续可升级为 OS keyring。
- 图像输入、transcript 文件输入、Gemini 多说话人、ElevenLabs 尚未实现（接口已留）。

## 5. 目录结构

```
src-tauri/src/
  lib.rs               # run() / 命令注册
  error.rs             # AppError
  config/mod.rs        # ApiKeys / LlmConfig / ConversationConfig / OutputConfig + load_settings
  extractor/mod.rs     # Extractor: extract_url/webpage/youtube/pdf, search_topic
  generator/mod.rs     # Generator + Provider + prompt 模板 + split_chunks
  tts/mod.rs           # TtsProvider trait + OpenAiTts + EdgeTts
  audio/mod.rs         # find_ffmpeg / concatenate_parts / reencode
  queue/mod.rs         # QueueManager / Task / commands
  queue/pipeline.rs    # run(): 四阶段管道 + 进度映射
src/
  lib/api.ts           # invoke 封装 + onTaskUpdate
  lib/types.ts         # 与 Rust 对齐的 TS 类型
  routes/+page.svelte  # 主布局（新建/任务/设置 三视图）
  routes/NewTask.svelte
  routes/TaskCard.svelte
  routes/Settings.svelte
```

### 运行期产物目录

- 根目录默认取系统应用数据目录（Linux `~/.local/share/com.podcastfy.ui/`、Windows `%APPDATA%\com.podcastfy.ui\`），可在设置页「输出目录」改写（持久化到 `output.json` / `OutputConfig`）。
- 任务目录名为可读前缀 `日期-主题-id前8位`（例：`2026-09-12-我的播客-90a5b181`），中文保留、符号剔除；`parts/` 中间产物在任务成功后清理，成品音频/视频/封面保留。
- 自定义输出目录后，asset 协议 scope 由 `allow_asset_dir` 动态放行。
- **但 `asset://` 不能直接当媒体源**：WebKitGTK 的媒体加载器只认 `file / http(s) / blob / data`，
  把 `asset://` URL 交给 `<audio>` / `<video>` 会稳定报 `MediaError #4`（协议放行、URL 编码、
  解码器三条都已排除）。因此媒体不要直接走 `convertFileSrc`，前端先 `fetch` 成 `Blob` 再
  `URL.createObjectURL` 播放：`blob:` 已在 CSP `media-src` 放行，`connect-src` 另需放行 `asset:`。
  `http://asset.localhost`（`dangerousUseHttpScheme`）只对 WebView2 / Android 有效，WebKitGTK 无此能力。
  图片预览不受影响，仍可直接用 `convertFileSrc`。
- 重启恢复：列表来自 `task_index.json` ∪ `tasks/*/` 目录扫描（后者覆盖本改动之前的历史任务；标题取可读目录名里的主题，纯 uuid 的旧目录回落到转录稿首句）；进程退出时还在进行中的状态载入时归一为 failed / `interrupted`，不会永远转圈。
- 删除任务会连带清掉它的产物（只删本任务记录过的已知文件与 `parts/`，不做整目录递归），否则下次启动扫描会把「已删除」的任务找回来。
