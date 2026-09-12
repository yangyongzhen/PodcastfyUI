# 后续路线：从播客音频到视频（L1–L4）

> **状态（2026-09-13）**：**L1 + L2 已落地并真机实测通过** —— 任务卡片上的「导出视频」把已完成的
> mp3 配成带中文标题的 mp4，横版/竖版由**一个配置项**决定。实测数据见「已完成（L1 + L2）」。
> L3（双人对白字幕）、L4（动态双人形象）仍是**计划**，本文只保留其论证。
>
> 本文是逐步推进的路线图：做完一档就把它移进「已完成」并补实测数据。

- **目的**：让产物不止是 mp3 —— 同一段对话能直接贴到视频号 / YouTube / B 站。
- **原则**：继续走「纯 Rust + 用户自装 ffmpeg」路线，不引本地 ML、不捆 GPL 编码器。

## 为什么做视频

现阶段的产物是 `podcast.mp3`。它能听，但**不能直接发**：短视频平台与视频号要的是视频文件，
而把音频配一张图做成视频，是这条链路上唯一还需要用户手工拿别的软件补的一步。
矩阵里缺的正是这一格。

## 现状盘点

| 能力 | 现状 |
|---|---|
| 音频产物 | ✅ 已完成，真实双人对谈 mp3 跑通 |
| ffmpeg 调用 | ✅ `audio::find_ffmpeg()` + `spawn_blocking`，已有「测试 FFmpeg」自检按钮 |
| 视频导出 | ✅ `src-tauri/src/video/mod.rs`：波形 / 静态封面、横竖两画幅、中文标题烧入 |
| 中文字体 | ✅ **随包** Droid Sans Fallback（`src-tauri/fonts/`，Apache-2.0），另支持自定义字体路径 |
| 界面 | ✅ 设置页「视频导出」；任务卡片「导出视频 / 打开视频文件」+ 应用内 `<video>` 预览 |

## 已跑通的样例命令

真机验证过的滤镜组合（`ffmpeg 6.1.1`，编码器 `libx264`/`aac` 齐备）：

```bash
# 波形（L2）
ffmpeg -i in.mp3 -filter_complex \
  "[0:a]showwaves=s=1280x720:mode=cline:colors=0x22D3EE|0x67E8F9:rate=25,format=yuv420p[v]" \
  -map "[v]" -map 0:a -c:v libx264 -crf 23 -c:a aac -shortest out.mp4

# 静态封面（L1，自绘渐变）
ffmpeg -f lavfi -i "gradients=s=1280x720:c0=0x1E1B4B:c1=0x0891B2:rate=25" -i in.mp3 \
  -filter_complex "[0:v]format=yuv420p[v]" -map "[v]" -map 1:a -c:v libx264 -crf 23 -shortest out.mp4
```

## 已完成（L1 + L2）

**效果**：任务卡片点「导出视频」，得一个能直接上传的 mp4 —— 画面是声波（或静态封面），
顶部压着标题与副标题；中文标题真正烧进画面，不依赖播放器字体。

**实现**（全部在 `src-tauri/src/video/mod.rs`，约 470 行）：

| 项 | 做法 |
|---|---|
| 画幅 | `aspect` 单选：横版 1280×720 / 竖版 720×1280（**不**同时出两版，避免翻倍耗时） |
| 波形 | `showwaves=mode=cline` + 品牌青双色，25 fps |
| 静态封面 | `cover=generated` 走 `gradients` 自绘品牌渐变；`cover=custom` 用自选图 `scale`+`crop` 铺满后居中裁切 |
| 文字 | `drawtext` 烧入标题/副标题；文本写进临时 `textfile`，所以中文、引号、冒号、逗号**都不用转义** |
| 字体 | 三级查找：`font_path` → 资源目录 `fonts/DroidSansFallbackFull.ttf` → 仓库内（开发期）；都没有则**提前报错** |
| 产物 | 横版 `video.mp4` / 竖版 `video-portrait.mp4`，落在任务目录，与 mp3 同级 |
| 进度 | 复用既有 `queue.update` → `task-update` 事件；新增 `TaskStatus::Exporting` |
| 失败 | 只记 `task.video_error`，状态回到 `completed`，**音频产物不受影响** |
| 取消 | 编码前检查一次；编码中不中断（与既有口径一致），收尾时不会把已取消的任务改回完成 |

**实测**（`/tmp/podcastfy-demo.mp3`：202.4 秒、2.32 MB 真实双人对谈）：

| 用例 | 产物 | 大小 | 编码耗时 |
|---|---|---|---|
| 横版波形 1280×720 | h264 + aac，202.4 秒 | 28.77 MB | 58.8 s |
| 竖版波形 720×1280 | h264 + aac，202.4 秒 | 47.72 MB | 60.2 s |
| 横版静态封面（自绘） | h264 + aac，202.4 秒 | 7.55 MB | 89.1 s |

（同源 25 秒片段复跑：横版 3.90 MB / 7.5 s、竖版 6.48 MB / 7.8 s、自选图封面 0.42 MB / 8.1 s。）

**验收证据**（`cargo run --example video_export_test`，与应用内导出同一条代码路径）：

- 4 组导出全部成功，`ffprobe` 分辨率/编码/时长符合预期；
- **中文确实烧入**：同配置再出一版无标题，比较画面顶部亮度 —— 有字 YAVG 36.01 vs 无字 16.00；
- **两条失败路径提前拦截**：封面图不存在、字体文件不存在，都返回可操作报错；
- **音频未被破坏**：整轮前后 mp3 大小与 md5（`958538ee…`）完全一致。

**成本提醒**：一段 3.4 分钟的音频，单次导出要 **约 1–1.5 分钟** CPU 编码。
所以导出**只手动触发**，不跟跑 —— 否则每次生成都要多付这份时间。

## L3 双人对白字幕

- **效果**：按讲话人分行显示字幕，两人不同色/不同位，随音频推进。
- **实现**：逐段 `parts/NNNN.mp3` 用 `ffprobe` 取时长累加出时间轴 → 生成 SRT/ASS → `subtitles`/`ass` 滤镜烧入。
- **依赖**：现成（`subtitles`、`ass` 滤镜已验证可用），只需时间轴累加不漂移。
- **成本**：无新依赖，改动集中在 `video/mod.rs` + 分段命名约定。
- **验收**：字幕出现时刻与音频听感对齐；长句折行不出画面；中英文混排不乱码。
- **风险**：分段边界的累积误差（每段 ffprobe 有毫秒级误差，段多了会漂）；副标题与波形抢位置。

## L4 动态双人形象

- **效果**：两个会随声音起伏的虚拟形象，像双人播客的动图封面。
- **实现**：预留两个静态立绘 + `overlay` 随音量缩放/位移，或直接用 `showspectrum` 做声纹柱。
- **依赖**：需要立绘素材（授权要干净，或自绘极简几何形象）。
- **成本**：素材 + 滤镜图复杂度上一个台阶。
- **验收**：两人动作分别跟随各自音轨（需要按讲话人切片，与 L3 共用时间轴）。
- **风险**：最容易翻车的一档 —— 素材授权、动作与声音不同步、渲染时间再翻倍。

## 授权与依赖

- **ffmpeg 不自带**：应用只做启动自检 + 引导用户安装。内置 `libx264` 是 GPL，与 Apache-2.0 上架冲突；走「用户自装」这条线最干净。
- **中文字体随包**：Droid Sans Fallback（Apache-2.0，可再分发），约 4.0 MB，声明在 `tauri.conf.json` 的 `resources: fonts/*`。
- 自选图封面由用户提供，不引入新的素材授权问题。

## 落点与改动面

| 位置 | 内容 |
|---|---|
| `src-tauri/src/video/mod.rs` | 滤镜图组装、字体解析、`export()`、字体状态命令 |
| `src-tauri/src/queue/mod.rs` | `TaskStatus::Exporting`、`video_path`/`video_error`、`export_video_task`/`open_video_file` |
| `src-tauri/src/queue/pipeline.rs` | 可选第五阶段 `export_video()`（失败不碰音频） |
| `src-tauri/src/config/mod.rs` | `VideoConfig` + `video.json` 读写 |
| `src/routes/Settings.svelte` | 画幅/风格/封面/标题/字体 + 字体状态 |
| `src/routes/TaskCard.svelte` | 导出按钮、应用内 `<video>` 预览、失败提示 |

## 建议顺序

1. ~~L1 静态封面~~ ✅
2. ~~L2 波形~~ ✅
3. L3 双人对白字幕（时间轴复用 L3 的 ffprobe 累加）
4. L4 动态形象（素材到位再做）

## 已拍板（2026-09-13）

- **画幅**：**一个配置项**在横版/竖版之间单选，不同时输出两版（避免一次导出付两遍编码时间）。
- **封面**：自绘渐变底图与自选图片**都支持**，用 `cover` 二选一。
- **字体**：中文字体**随包**，同时允许用户自定义 `font_path`。
- **批量导出**：不做；仍是任务卡片上逐任务手动触发。

## 关联文档

- [架构](architecture.md) · [模块职责](modules.md) · [命令 API](api.md) · [开发日志](devlog.md)
- [项目介绍文章](introducing-podcastfyui.md)
