/**
 * 轻量 i18n：以中文原文为 key，英文查表替换。
 *
 * 设计取舍：
 * - 不做 key 抽离（避免 160+ 处字符串大改），直接用中文原文当 key，
 *   迁移时可逐文件包 t(...)，未收录的字符串自动回落中文，不会出现空文案。
 * - locale 跟随系统语言，用户可在设置页覆盖，选择持久化到 localStorage。
 */

export type Locale = "zh-CN" | "en";

const STORAGE_KEY = "podcastfyui.locale";

export const LOCALES: { id: Locale; label: string }[] = [
  { id: "zh-CN", label: "简体中文" },
  { id: "en", label: "English" },
];

/** 英文词典：中文原文 → 英文译文。迁移过程中按文件逐步补齐。 */
const EN: Record<string, string> = {
  // 通用
  保存: "Save",
  取消: "Cancel",
  确认: "Confirm",
  确定: "OK",
  删除: "Delete",
  关闭: "Close",
  关闭通知: "Dismiss notification",
  重试: "Retry",
  加载中: "Loading",
  // 播放器
  播放: "Play",
  暂停: "Pause",
  播放进度: "Playback progress",
  播放速度: "Playback speed",
  // 语言
  语言: "Language",
  界面语言: "Interface language",

  // 视频导出（TaskCard / Settings）
  导出视频: "Export video",
  "导出中…": "Exporting…",
  "视频已导出": "Video exported",
  "打开视频文件": "Open video file",
  "视频导出失败：{err}": "Video export failed: {err}",
  视频导出: "Video export",
  画幅: "Aspect ratio",
  "横版 16:9": "Landscape 16:9",
  "竖版 9:16（短视频）": "Portrait 9:16 (short video)",
  画面风格: "Visual style",
  "波形（推荐）": "Waveform (recommended)",
  静态封面: "Static cover",
  封面来源: "Cover source",
  自动生成渐变底图: "Auto-generated gradient",
  自选图片: "Custom image",
  封面图片: "Cover image",
  "选择图片…": "Choose image…",
  标题: "Title",
  留空则使用任务标题: "Leave empty to use the task title",
  副标题: "Subtitle",
  留空则使用播客标语: "Leave empty to use the podcast tagline",
  "自定义中文字体（可选）": "Custom CJK font (optional)",
  留空则使用随包字体: "Leave empty to use the bundled font",
  "选择字体文件…": "Choose font file…",
  "当前中文字体：{path}": "Current CJK font: {path}",
  "字体不可用：{err}": "Font unavailable: {err}",
  "视频由 ffmpeg 编码，导出在任务卡片上手动触发，不会随生成自动执行。":
    "Videos are encoded by ffmpeg; export is triggered manually from the task card and never runs automatically.",
  图片: "Image",
  字体: "Font",

  // NewTask.svelte
  "选择文件失败：{err}": "Failed to select files: {err}",
  "请至少提供 URL、PDF、文本或主题之一": "Provide at least one of: URL, PDF, text, or topic",
  "任务已创建": "Task created",
  "创建任务失败：{err}": "Failed to create task: {err}",
  "新建播客": "New podcast",
  "网页 / YouTube 链接（每行一个）": "Web / YouTube links (one per line)",
  "本地 PDF（可选）": "Local PDF (optional)",
  "选择 PDF 文件…": "Choose PDF files…",
  "可多选": "Multiple allowed",
  "移除 {p}": "Remove {p}",
  "纯文本（可选）": "Plain text (optional)",
  "粘贴文章内容…": "Paste article content…",
  "主题（联网搜索扩展，可选）": "Topic (web-search expansion, optional)",
  "如：量子计算的最新进展": "e.g. latest advances in quantum computing",
  "长篇模式（30 分钟+，多轮讨论）": "Long-form mode (30 min+, multi-round)",
  "启动中…": "Starting…",
  "🎙️ 生成播客": "🎙️ Generate podcast",

  // +page.svelte
  "新建": "New",
  "任务": "Tasks",
  "任务列表加载失败：{err}": "Failed to load task list: {err}",
  "（{n} 进行中）": " ({n} running)",
  "设置": "Settings",
  "还没有可用的 API 密钥，现在提交任务会在生成阶段失败。":
    "No usable API key yet — tasks submitted now will fail during generation.",
  "去设置": "Open settings",
  "使用说明": "How to use",
  "先在「设置」里填好 API 密钥（LLM 与 TTS 至少各一个；TTS 选 edge 则免 key）":
    "First fill in API keys under “Settings” (at least one for LLM and one for TTS; edge TTS needs no key)",
  "回到「新建」，粘贴网页 / YouTube 链接，或填主题 / 纯文本 / PDF":
    "Back to “New”, paste web / YouTube links, or fill in topic / plain text / PDF",
  "点「生成播客」，到「任务」页看实时进度":
    "Click “Generate podcast”, then watch live progress on the “Tasks” page",
  "完成后可直接播放、编辑转录稿、删除任务":
    "When done, play the audio, edit the transcript, or delete the task",
  "需要系统安装": "Requires system-installed",
  "（音频拼接用）。": " (used for audio concatenation).",
  "搜索标题…": "Search titles…",
  "按标题搜索任务": "Search tasks by title",
  "按状态筛选": "Filter by status",
  "全部": "All",
  "进行中": "Running",
  "已完成": "Completed",
  "失败": "Failed",
  "已取消": "Cancelled",
  "历史": "History",
  "失败 / 已取消": "Failed / Cancelled",
  "还没有播客任务": "No podcast tasks yet",
  "粘贴网页 / YouTube 链接、选一份 PDF，或直接输入主题，就能生成双人对话播客。":
    "Paste web / YouTube links, pick a PDF, or just enter a topic to generate a two-host podcast.",
  "开始创建第一个播客": "Create your first podcast",
  "没有匹配的任务（试试清空搜索或换筛选条件）。":
    "No matching tasks (try clearing the search or changing the filter).",

  // Settings.svelte
  "API 密钥": "API keys",
  "密钥以明文保存在本机应用数据目录，仅靠文件权限保护；除你配置的服务商外不会发往别处，共享电脑请谨慎使用。":
    "Keys are stored as plaintext in this app's local data directory, protected only by file permissions. They are never sent anywhere except the providers you configure — be careful on shared machines.",
  "ElevenLabs（预留）": "ElevenLabs (reserved)",
  "预留": "Reserved",
  "Serper（主题搜索，可选）": "Serper (topic search, optional)",
  "不填则用 DuckDuckGo": "Leave blank to use DuckDuckGo",
  "LLM（转录稿生成）": "LLM (transcript generation)",
  "openai（兼容任意 OpenAI 接口）": "openai (any OpenAI-compatible endpoint)",
  "ollama（本地）": "ollama (local)",
  "OpenAI 密钥": "OpenAI API key",
  "Anthropic 密钥": "Anthropic API key",
  "Gemini 密钥": "Gemini API key",
  "本地 Ollama 无需密钥。": "Local Ollama needs no API key.",
  "其它（可选）": "Other (optional)",
  "Base URL（可选，Ollama 默认 http://localhost:11434/v1）":
    "Base URL (optional; Ollama default http://localhost:11434/v1)",
  "播客人设": "Podcast persona",
  "播客名": "Podcast name",
  "输出语言": "Output language",
  "English / 中文": "English / Chinese",
  "简体中文": "Simplified Chinese",
  "繁體中文": "Traditional Chinese",
  "主持人 1 角色": "Host 1 role",
  "主持人 2 角色": "Host 2 role",
  "风格（逗号分隔）": "Style (comma-separated)",
  "互动技巧（逗号分隔）": "Engagement techniques (comma-separated)",
  "创造力 0–2": "Creativity 0–2",
  "长片最大轮数": "Max long-form rounds",
  "自定义指令": "Custom instructions",
  "可选，追加给 LLM 的指令": "Optional instructions appended to the LLM",
  "语音（TTS）": "Voice (TTS)",
  "默认 TTS": "Default TTS",
  "edge（免费）": "edge (free)",
  "OpenAI 主持人 1 音色": "OpenAI host 1 voice",
  "OpenAI 主持人 2 音色": "OpenAI host 2 voice",
  "Edge 主持人 1": "Edge host 1",
  "Edge 主持人 2": "Edge host 2",
  "豆包 App ID": "Doubao App ID",
  "豆包 Access Token": "Doubao Access Token",
  "豆包 Resource ID": "Doubao Resource ID",
  "豆包 API 版本": "Doubao API version",
  "豆包（火山引擎）": "Doubao (Volcano Engine)",
  "豆包集群": "Doubao cluster",
  "豆包主持人 1 音色": "Doubao host 1 voice",
  "豆包主持人 2 音色": "Doubao host 2 voice",
  "仅在选用豆包 TTS 时需要填写以下豆包凭证。": "The credentials below are only required when you use Doubao TTS.",
  "已保存": "Saved",
  "保存中…": "Saving…",
  "保存设置": "Save settings",
  "重新加载": "Reload",
  "连通性自检": "Connectivity check",
  "测试中…": "Testing…",
  "测试 LLM": "Test LLM",
  "测试 TTS": "Test TTS",
  "测试 FFmpeg": "Test FFmpeg",
  "走真实请求路径，仅消耗一次极小额度":
    "Uses the real request path; consumes one tiny call",

  // TaskCard.svelte
  "排队中": "Queued",
  "抽取内容": "Extracting",
  "生成对话稿": "Generating script",
  "语音合成": "Synthesizing speech",
  "音频合成": "Muxing audio",
  "转录稿加载失败：{err}": "Failed to load transcript: {err}",
  "转录稿已保存": "Transcript saved",
  "保存失败：{err}": "Save failed: {err}",
  "将用当前转录稿重新合成音频，并覆盖已有音频文件。继续吗？":
    "This re-synthesizes audio from the current transcript and overwrites the existing file. Continue?",
  "仅重新合成音频": "Re-synthesize audio only",
  "开始合成": "Start synthesis",
  "已开始重新合成，进度见本卡片": "Re-synthesis started; see progress on this card",
  "重新合成失败：{err}": "Re-synthesis failed: {err}",
  "已请求取消该任务": "Cancellation requested",
  "删除任务「{title}」？该任务的转录稿与音频文件会一并删除，且不可恢复。":
    "Delete task “{title}”? Its transcript and audio file will be deleted permanently.",
  "删除任务": "Delete task",
  "任务已删除": "Task deleted",
  "删除失败：{err}": "Delete failed: {err}",
  "{n} 秒": "{n} sec",
  "{n} 分钟": "{n} min",
  "{h} 小时 {m} 分": "{h} h {m} min",
  "生成进度": "Generation progress",
  "预计还需约 {eta}": "About {eta} left",
  "后退10秒": "Back 10 seconds",
  "前进10秒": "Forward 10 seconds",
  "倍速，当前 {rate}×，点击切换": "Speed, currently {rate}×, click to change",
  "收起转录稿": "Hide transcript",
  "查看/编辑转录稿": "View/edit transcript",
  "打开音频文件": "Open audio file",
  "合成中…": "Synthesizing…",
  "每行格式：": "Format per line:",
  "PERSON_1: 台词": "PERSON_1: text",
  "或": "or",
  "PERSON_2: 台词": "PERSON_2: text",
  "；没有前缀的行合成时会被跳过。": "; lines without a prefix are skipped during synthesis.",
  "{n} 条": "{n} lines",
  "共 {n} 条对白 / {m} 字": "{n} lines / {m} chars",
  "有 {n} 行缺少": "{n} lines are missing the",
  " 前缀，合成语音时会被跳过（下面用虚线标出）。":
    " prefix; skipped in speech synthesis (shown with a dashed line below).",
  "（暂无转录稿）": "(No transcript yet)",
  "编辑转录稿": "Edit transcript",
  "保存修改": "Save changes",
  "取消编辑": "Cancel editing",
};

function detectLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "zh-CN" || saved === "en") return saved;
  } catch {
    /* localStorage 不可用时按系统语言 */
  }
  const nav = typeof navigator !== "undefined" ? navigator.language : "zh-CN";
  return nav.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

let current = $state<Locale>(detectLocale());

export function getLocale(): Locale {
  return current;
}

export function setLocale(next: Locale): void {
  current = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    /* 持久化失败不影响本次会话 */
  }
}

/** 取当前 locale 下的文案；params 用 {name} 占位替换。 */
export function t(source: string, params?: Record<string, string | number>): string {
  let out = current === "en" ? (EN[source] ?? source) : source;
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      out = out.replaceAll(`{${key}}`, String(value));
    }
  }
  return out;
}
