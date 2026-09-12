# 开发日志

## 2026-09-12 · 端到端冒烟测试（4/4 通过）

### 方法
本机网络无法直连 LLM/OpenAI/Edge TTS（Edge 端点返回 400 "services aren't available"），
故采用「真实生产代码 + 本地 mock」组合，完整走 `examples/smoke_test.rs`：

| 阶段 | 组件 | 真实/mock |
|---|---|---|
| 0 | 本地 mock LLM 服务（tokio TCP，实现 OpenAI `/v1/chat/completions` 协议） | mock |
| 1 | `Extractor::extract_url` 抓 https://example.com（Readability 抽取） | **真实** |
| 2 | `Generator::generate_qa`（OpenAI 兼容 client → mock LLM，写 transcript.txt） | **真实** |
| 3 | 实现生产 `TtsProvider` trait 的 mock（ffmpeg sine 生成每句 mp3，双角色不同频率） | trait 真实 / 音频 mock |
| 4 | `audio::concatenate_parts`（ffmpeg concat demuxer → 24kHz mono mp3） | **真实** |

### 结果
```
[0] mock LLM at http://127.0.0.1:45449/v1
[1] extraction OK (145 chars): "Title: Example Domain..."
[2] generation OK (402 chars, 9 lines)
[3] tts OK (4 lines -> 4 mp3 parts)
[4] concat OK -> /tmp/podcastfy_smoke_1836032/podcast.mp3 (88173 bytes)
SMOKE PASS — 4/4 stages green
```
ffprobe 验证最终产物：`mp3 / 24000 Hz / mono / 7.32s / ~96kbps` ✅
转录稿格式正确（播客头尾 + `PERSON_1:/PERSON_2:` 对话行 + ending message）✅

复跑方式：`cd src-tauri && cargo run --example smoke_test`

### 过程中发现并修复的真 bug
1. **网页抽取 fallback 丢正文**（extractor/mod.rs）：页面无 `<article>/<main>` 时，
   fallback 分支取到空串后直接丢弃了 Readability 已抽到的正文，只剩标题头（48 字符）。
   修复：两个来源取**较长者**。影响面：所有无 article/main 标签的页面（如 example.com）。
2. `split_transcript` 从 pipeline 私有函数提升为 `queue::split_transcript` 公开复用。

### 环境记录
- 本机磁盘曾 100% 满（0 字节）导致链接器 SIGBUS / "No space left on device"，
  已清理本项目 `target/`（cargo clean，5.6G）；后续大编译前先 `df -h /` 确认 ≥3G 空闲。
- Edge TTS 端点从本网络不可达（微软侧限制，非代码问题）；代码内错误提示已引导切换 OpenAI。
- 真实 LLM/TTS 的端到端验证（含 UI 内任务流）留待有 API key 时执行。

## 2026-09-12 · 路线确认 + 全量 MVP 落地

### 决策
- 选定**路线 B：Tauri v2 + 纯 Rust 复刻**（弃用 Python sidecar 过渡方案）。
  理由：上游核心无本地 ML，流水线 = HTTP 编排 + 文本处理 + ffmpeg，Rust 可完整覆盖；
  分发为单二进制（+ffmpeg），无 Python 运行时包袱。

### 完成
- [x] Tauri v2 + Svelte 5 (runes) + SvelteKit static 脚手架
- [x] 配置模块：`api_keys.json`（0600）/ `llm.json` / `conversation.yaml`
- [x] 抽取模块：网页（Readability 0.3 + scraper 兜底）、YouTube 字幕
      （解析 `ytInitialPlayerResponse` → timedtext XML）、PDF（pdf-extract）、
      主题搜索（Serper → DuckDuckGo 兜底）
- [x] 对话稿生成：OpenAI 兼容 / Anthropic / Gemini / Ollama 四 provider；
      短片单请求、长片分块滚动多轮（上一轮尾部 4000 字回喂）
- [x] TTS：`TtsProvider` trait + 工厂；OpenAI（tts-1-hd）与 Edge（免费，免 key）
- [x] 音频：ffmpeg sidecar 拼接（concat demuxer → 24kHz mono mp3），
      可执行文件旁 / PATH 自动发现
- [x] 任务系统：`QueueManager`（`Arc<Mutex<HashMap>>` 共享，clone 轻量）+
      四阶段进度（0-15 抽取 / 15-50 生成 / 50-90 TTS / 90-100 拼接）+
      `task-update` 事件
- [x] 前端：新建表单 / 任务列表（实时进度条、取消、删除）/ 转录稿查看与编辑 /
      `<audio>` 播放（convertFileSrc）/ 设置面板（keys、LLM、人设、音色）
- [x] 验证：`cargo check` ✅、`cargo test` 3 passed ✅、
      `svelte-check` 0 errors 0 warnings ✅、`vite build` ✅

### 踩坑记录
- `readability` crate 最新 0.3.0，API 是 `extractor::extract(&mut bytes, &Url) -> Product{title,content,text}`，
  不是 `Readability::new_document`（网上很多教程写的是旧 API）。
- `pdf-extract::extract_text` 是**同步单参**函数（只收 path），CPU 密集需 `spawn_blocking`。
- `reqwest::RequestBuilder::json()` 只能调一次，Gemini 分支先构造 `Value` 再一次性 `.json(&body)`。
- Svelte 5 runes 模式：`export let` 必须改 `$props()`；`#each` 循环变量传 prop
  要写 `task={t}` 而非 `{task}`；`$derived` 才能保证派生列表响应式。
- 进度回调闭包要求 `'static`：捕获 `QueueManager` 的轻量 clone（内部 `Arc<Mutex>`），
  而非借用。
- 本机环境走 USTC crates 镜像（`ustc` index），依赖解析正常。

### 待办（下一迭代）
- [ ] ElevenLabs TTS provider（多语言效果最好）
- [ ] Gemini multi-speaker（一次请求出自然双人对话，替代逐句拼接）
- [ ] 图像输入（多模态 LLM 描述后入对话稿）
- [ ] transcript 文件直接输入（跳过抽取+生成，直接 TTS）
- [ ] OS keyring 存储 API keys（替代 JSON 文件）
- [ ] 任务持久化（重启后恢复历史任务；当前仅内存态）
- [ ] ffmpeg 随包分发（tauri externalBin）+ 三平台打包签名
- [ ] Edge TTS 协议失效监控（改 WebSocket 版本）
- [ ] 集成测试：mock LLM/TTS 端点跑通 pipeline 四阶段
