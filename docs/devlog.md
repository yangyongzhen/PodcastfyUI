# 开发日志

## 2026-09-13 · 播放器播不出声：WebKitGTK 的媒体管线不认 `asset://` 自定义 scheme

### 现象
用户实测点播放，音频和视频**都**加载失败：`音频 #4 asset://localhost/%2Froot%2F…%2Fpodcast.mp3`
（`MediaError #4` = 该来源格式/协议不受支持）。

### 排除法（每条都有证据，不是推测）
| 假设 | 反证 |
|---|---|
| asset 协议 scope 没放行这条路径 | Tauri 自身单测写明 `allow_directory(dir, true)` 后 `dir/inner/folder/anyfile` 必须为 true；`escaped_pattern_with` 拼出的是规范的 `dir/**`（不存在 `dir**` 这种形态）；本机整条路径链无符号链接（`readlink -f` 与索引一致）⇒ 放行判定无辜 |
| URL 整体百分号编码是 bug | Tauri 2.11.5 的 handler 会 `percent_decode(path[1..])`，`%2F` 正确还原为 `/` |
| 缺 MP3 解码器 | `libgstmpg123.so` 在、`libmpg123.so.0` 依赖解析正常、playback / typefind / id3demux 齐全 |
| 缺 H.264 解码器导致音频也挂 | 两种格式同时失败，一个编解码器解释不了 |

### 根因
WebKitGTK 的**媒体加载器**只认 `file / http(s) / blob / data`，不认应用注册的自定义 scheme：
`asset://` 能被 `fetch` 正常取到字节（普通资源加载路径支持自定义 scheme），但把同一个 URL 交给
`<audio>` / `<video>` 的媒体管线，必然报 `#4`（源不受支持）。
`dangerousUseHttpScheme` / `http://asset.localhost` 是 WebView2（Windows）和 Android 的能力，
WebKitGTK 没有——在 tauri 2.11.5 的 Rust 源码里搜不到这个开关。

### 修复
1. `tauri.conf.json` CSP：`connect-src` 增加 `asset:`。原来只放行了 `media-src` / `img-src`，
   前端去 `fetch` 会被 CSP 拦掉。
2. `TaskCard.svelte`：播放源改成「`fetch` 取字节 → `Blob` → `URL.createObjectURL`」，
   交给媒体元素的是 `blob:`（在 `media-src` 里本就放行）；取字节失败则退回直连 URL，
   并把真实错误（`fetch` 报的 HTTP 码 / 媒体报的 `MediaError` 码）写进播放器下方常驻错误行。
3. 播放按钮在资源就绪前禁用，`<audio>` 就绪前不带 `src`，不再白触发一次注定失败的加载。
4. 播放器门槛由 `status === "completed"` 放宽为「有音频就渲染」：任务被归一成 `interrupted`
   （例如应用重启打断导出）后，产物其实还在，不该连播放入口都看不到。

### 验证
`svelte-check` 0 errors / 0 warnings；dev 重建 `Finished dev profile`；二进制时间（15:13:35）晚于
配置改动（15:13:19）⇒ 新 CSP 确实编进去了；日志显示 HMR 已更新 `TaskCard.svelte`。
**出声与否仍需用户点一次确认**（助手听不到声音，不得代验）。

### 未解：视频预览
视频除上面这条协议限制外，本机还缺 H.264 解码器（gst 插件目录无 `libgstlibav.so` /
openh264 / vaapi，`fakevideosink` 也缺），两条障碍叠加。用户决定先只修音频、视频搁置。

## 2026-09-13 · 任务队列持久化：重启不再丢列表，历史产物目录自动恢复

### 背景
任务表原来只在内存（`QueueManager` 一个 `HashMap`），应用一关列表就空了：上次生成的 mp3 / 转录稿
明明还在 `tasks/<uuid>/` 里，界面上却找不到，连点播放、打开目录复用都做不到——只能自己进
`~/.local/share/com.podcastfy.ui/tasks/` 翻。

### 完成
- **索引落盘**：新增 `task_index.json`（`{app_data_dir}/task_index.json`，载荷 `{version, tasks[]}`，与产物目录
  `tasks/` 命名不冲突）；`QueueManager::with_store(app_dir)` 取代 `new()`，在 `lib.rs` 的 `setup` 里用
  `app.path().app_data_dir()` 接线。`start` / `update` / `cancel` / `delete` / `save_transcript` 所有变更点写后即存，
  采用 `.tmp` → `rename` 原子写（崩溃不会留半截 JSON）；索引缺失、读写或解析失败都只 `warn`，绝不影响任务本身。
- **两路恢复**（缺一不可）：① 读索引；② 扫描 `tasks/*/` 重建没有索引记录的历史任务。三个产物都没有的空壳
  目录直接跳过，不在列表里留空卡片；只有转录稿/视频的目录保留为 failed，让「那次没跑完」看得见。
- **标题回落**：新式可读目录名 `日期-主题-短id` 取中间那段主题；纯 uuid 的旧目录取转录稿首句（跳过
  `PODCASTIFY` 与 tagline 两行表头，截断 42 字），都没有则给「历史任务 <id前8位>」。实测历史任务
  `90a5b181` 恢复出的标题是「欢迎回到 PODCASTIFY——你的私人生成式AI播客！我是今天的主持人，今天我们…」。
- **状态归一**：进程退出后 `pending/extracting/generating/synthesizing/muxing/exporting` 不可能再推进，
  载入时统一改 failed + `stage = "interrupted"` 并写明原因，避免界面永远转圈。
- **删除语义补齐**：原来 `delete` 只从内存摘掉、产物留在盘上；加了扫描恢复后会「删了又复活」，
  现在连带清产物（只删本任务记录过的已知文件与 `parts/`，不做整目录递归，索引被改坏也不会误删）。

### 验证（无头 + 实机）
- `cargo check` 0 errors（限并行 2 + 关 debuginfo）。
- `cargo test --lib`：**24 passed / 0 failed**（原 20 + 新增 4：目录恢复、uuid 目录标题回落、中断归一、删除不复活）。
- 实机重启 dev，日志两次打印恢复行：首次 `restored 1 task(s) (1 from workspace, 0 interrupted)`——走扫描，
  磁盘上只有产物的历史任务 `90a5b181` 被重建；二次 `restored 1 task(s) (0 from workspace, 0 interrupted)`——走索引，
  证明索引往返有效。`task_index.json` 874 字节 / 含 1 条任务，音频、转录稿、视频三个路径齐全。
- 环境提醒：本机内存仅 1.9G，dev 会话必须带 `CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0`，且用 `setsid`
  派生，否则会随父 shell 一起死掉（本轮踩过一次：应用、vite、cargo 全没了）。

## 2026-09-12 · 多平台产物目录 + 任务页三处修复

- 任务页三修：① 开 `assetProtocol`（scope `$APPDATA/**`），播放失败弹提示；② `RUNNING_STATUS` 补 `"exporting"`，视频预览改点击加载（`preload="none"`）；③ 两个「打开」命令改为尽力打开 + 始终回传绝对路径。
- 新增「输出目录」设置项（`output.json` / `OutputConfig`）：默认仍走系统应用数据目录，可改到桌面/文档；一旦自定义，`allow_asset_dir` 同步放行 asset 协议 scope，新目录下的音频/视频照样能播。
- 任务目录改可读前缀 `日期-主题-id前8位`：`slugify` 保留中文、剔符号与 emoji、限长、规避 Windows 保留名；`export_video` 由 `Task.audio_path` 反推任务目录，与 `execute` 口径一致。
- 任务成功后清理 `parts/` 中间产物（成品音频/视频/封面保留）。
- 新增 6 个命名逻辑单测：`cargo test --lib` 20 passed / 0 failed；`cargo run --example smoke_test` 4/4 阶段绿。
- 环境记录：本机内存仅 1.9G，dev 全量构建若并行 5 个 rustc 会耗尽内存卡死（cargo 进 D 状态）；改 `CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0`（test profile 同步关 debuginfo）后可正常构建。

## 2026-09-12 · 主题搜索兜底链：Exa 托管 MCP 免密钥可用（国内可达）

### 背景
「只给一个主题让应用去搜资料」这条路径在本机一直是死的：DuckDuckGo 解析到 IPv6，而本机没有
v6 默认路由 → 必失败/挂起；Google 系在国内不通；taotoken 网关虽然有 `/v1/search` 端点，但现有
key 是 **401（无该权限）**。用户提出参考 atomcode `web_search`（AI 搜索）的思路，并点名 Exa。

### 实测（本机，任何 HTTP 码＝可达）
| 后端 | 实测 | 结论 |
|---|---|---|
| `mcp.exa.ai/mcp`（MCP over Streamable HTTP） | `initialize` / `tools/list` / `tools/call` 全 200，**不带 key**；中文主题 2.6s 返回 5073 字中文正文 | ✅ **免密钥可用**（DNS 纯 IPv4，无 v6 坑） |
| `api.exa.ai/search`（REST） | 402 | ❌ 需付费 key（且 DNS v6 优先） |
| 博查 / 智谱 / 千帆 | 405 / 401 / 401 | ✅ 可达，需各自 key |
| Google Serper（代理） | 403 | ✅ 可达，需 key |
| taotoken `/v1/search` | 401（同一 key 的 `/v1/models` 是 200） | ⚠️ 端点存在但该 key 无权限 |
| DuckDuckGo | 超时 | ❌ 本机不可达 |

### 完成
- `extractor/mod.rs`：新增 `SearchCfg`（运行期视图）+ `search_topic()` 兜底链，`auto` 顺序为
  **exa → serper → bocha → zhipu → qianfan → ddg**，没配 key 的商业后端自动跳过；新增 Exa MCP
  （`tools/call` + SSE/纯 JSON 双解析 + `isError` 处理）与博查 / 智谱 / 千帆三个 REST provider；
  各后端 15-25s 短超时 + 明确错误文案（不再吐裸 reqwest builder error）。
- 配置：`SearchConfig` + `search.json`（`provider` / `num_results` / `degrade_without_search`）+
  `get_search_config` / `save_search_config`；`ApiKeys` 增 `exa` / `bocha` / `zhipu` / `qianfan`。
- 管道：搜索全失败且 `degrade_without_search`（默认开）时**降级继续** —— 素材里写入
  「live web search was unavailable」并要求模型在开场说明未联网核实；关掉则维持任务失败。
- 前端：设置页「主题搜索」区块（后端下拉 / 条数 / 降级开关 / 5 个可选密钥）+ `SearchConfig`
  TS 镜像 + i18n 词条。

### 验证
- `cargo run --example search_probe`（**全部 key 为空**、中文主题）：`SEARCH PROBE PASS`，
  auto → Exa，1.7s 拿到 17043 字符真实中文正文。
- `cargo check` exit=0；`cargo test --lib` 14 passed / 0 failed；`svelte-check` 0 errors / 0 warnings。
- 新增探针 `src-tauri/examples/search_probe.rs`（可传主题与后端名复跑）。

### 结论
搜索链从「本机必失败」变成「零凭证即可用」，且不存在单点：有 key 的商业后端是可选取代，
DuckDuckGo 只在其它都失败时兜底；全都失败还有「降级继续」保底。

## 2026-09-13 · 视频导出（L1 静态封面 + L2 波形）落地 + 真机实测

### 目标
把已完成的 mp3 直接变成能上传的 mp4（波形/封面画面 + 中文标题烧入），横版竖版由**一个
配置项**决定，且不跟跑 —— 手动触发，避免每次生成都多付一份编码时间。路线见 `docs/roadmap.md`。

### 完成
- `src-tauri/src/video/mod.rs`：滤镜图组装、前置校验（ffmpeg / 音频 / 字体 / 封面图）、
  `export()`、字体三级查找（自定义 → 资源目录 → 仓库内）。
- 配置：`VideoConfig` + `video.json`（`get_video_config` / `save_video_config`）。
- 队列：`TaskStatus::Exporting`、`video_path` / `video_error`、`export_video_task` /
  `open_video_file`；`pipeline::export_video` 作为**可选第五阶段**，失败只写 `video_error`，
  音频产物不受影响。
- 前端：设置页「视频导出」区块（画幅 / 风格 / 封面 / 标题 / 字体 + 字体状态 +
  「选择字体文件…」）；任务卡片加「导出视频 / 打开视频文件」、应用内 `<video>` 预览与失败提示。
- 中文字体随包：`src-tauri/fonts/DroidSansFallbackFull.ttf`（4.0 MB，Apache-2.0），
  `tauri.conf.json` 声明 `resources: fonts/*` —— 系统里只有 1 个中文字体，不随包会出方块字。

### 实测（真实双人对谈 mp3：202.4 秒 / 2.32 MB）
| 用例 | 分辨率 | 产物 | 编码耗时 |
|---|---|---|---|
| 横版波形 | 1280×720 | 28.77 MB | 58.8 s |
| 竖版波形 | 720×1280 | 47.72 MB | 60.2 s |
| 横版静态封面（自绘渐变） | 1280×720 | 7.55 MB | 89.1 s |

复跑：`cd src-tauri && cargo run --example video_export_test`（`EP_AUDIO` 可换输入音频）。

### 结论（`VIDEO EXPORT PASS`）
- 4 组导出 `ffprobe` 分辨率 / 编码 / 时长全部符合预期；两条失败路径（封面图不存在、
  字体不存在）被提前拦截，而不是产出一个坏 mp4。
- **中文确实烧进画面**：同配置再出一版无标题做对照，画面顶部亮度（YAVG）36.01 vs 16.00。
- **音频未被破坏**：整轮导出前后 mp3 大小与 md5 完全一致（全程只读输入）。

### 踩坑记录
- `metadata=print`（取 `lavfi.signalstats.YAVG`）的输出走 ffmpeg 的 **info** 日志：
  命令里带 `-v error` 会把要解析的数值一起吞掉，取证脚本静默拿到空值 —— 看起来像
  「代码没画字」，实际是取证方式错了。
- 一次 debug 代码生成 + 增量产物吃掉约 2G 磁盘（根分区一度只剩 731M）：大编译前先
  `df -h /`；紧张时清本项目 `target/debug/incremental`（**不**动 `deps/*.rlib`）。
- 竖版产物比横版大 66%（47.72 MB vs 28.77 MB），同为 25 fps：竖版整帧都是波形运动区域，
  x264 更难压（推测，未单独做对照实验）。要控体积应对竖版单独降 crf 或降帧率。
- 自绘渐变封面反而比波形慢（89.1 s > 58.8 s）：`gradients` 是逐帧重新生成的源。

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
