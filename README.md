# PodcastfyUI

用 **Tauri v2 + 纯 Rust** 复刻 [podcastfy](https://github.com/souzatharsis/podcastfy)（NotebookLM 播客功能的开源版）的桌面 GUI：把网页 / PDF / 纯文本 / 主题变成**双人对话播客音频**。

## 功能

- 输入：网页 URL、YouTube 链接（取字幕）、本地 PDF、纯文本、主题（联网搜索扩展）
- 对话稿：OpenAI 兼容 / Anthropic / Gemini / Ollama（本地）四种 LLM，支持短片（2-5 min）与长篇（30+ min 多轮）
- 语音：OpenAI TTS（tts-1-hd）或 Edge TTS（免费免 key），双主持人不同音色
- 任务队列：实时进度条、阶段提示、取消、删除
- 转录稿：完成后可查看、编辑、保存
- 音频：内置播放器直接听，或打开文件

## 运行

前置：Rust 工具链、Node 18+、**系统安装 ffmpeg**（音频拼接必需）。

```bash
# 首次
npm install

# 开发模式（热重载）
npm run tauri dev

# 生产构建
npm run tauri build
```

## 使用步骤

1. 打开应用 → **设置**：
   - 至少填一个 LLM key（如 OpenAI）；TTS 选 `edge` 则无需任何 key
   - Ollama 用户：provider 选 `ollama`，model 填本地模型名（默认连 `http://localhost:11434/v1`）
   - 按需改播客名、角色、语言、音色，点保存
2. **新建**：粘贴链接（每行一个）或填主题/文本，可勾选长篇模式 → 「生成播客」
3. **任务**页：看实时进度；完成后点播放，或「查看/编辑转录稿」
4. 转录稿改完保存后，当前版本需重新跑任务才会重新合成音频（后续版本将支持「仅重新合成」）

> 配置文件位置：`~/.local/share/com.podcastfy.ui/`（Linux；其它平台见 Tauri app_data_dir 文档）

## 文档

- [docs/architecture.md](docs/architecture.md) — 架构设计与流水线
- [docs/modules.md](docs/modules.md) — 各模块说明
- [docs/api.md](docs/api.md) — Tauri 命令 / 事件 API
- [docs/devlog.md](docs/devlog.md) — 开发日志与踩坑记录

## 状态

MVP 已完成（2026-09-12）：`cargo check` ✅ · `cargo test` ✅ · `svelte-check` 0 错 0 警 ✅ · `vite build` ✅

待办见 [devlog](docs/devlog.md)：ElevenLabs / Gemini multi-speaker / 图像输入 / keyring / 任务持久化 / 打包分发。

## License

Apache-2.0（与上游 podcastfy 一致）
