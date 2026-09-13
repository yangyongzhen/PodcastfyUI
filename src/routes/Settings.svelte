<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import * as api from "../lib/api";
  import type { ConversationConfig, LlmConfig, OutputConfig, SearchConfig, VideoConfig } from "../lib/types";
  import { t, LOCALES, getLocale, setLocale, type Locale } from "../lib/i18n.svelte.js";
  import { toast } from "../lib/ui/feedback.svelte.js";

  let keys = $state({ openai: "", anthropic: "", gemini: "", elevenlabs: "", serper: "", exa: "", bocha: "", zhipu: "", qianfan: "", doubao_app_id: "", doubao_access_token: "", doubao_resource_id: "volc.service_type.10029", doubao_api_version: "v1" });
  // 与后端 SearchConfig 默认值对齐：读盘失败时保留这份默认值。
  let search = $state<SearchConfig>({
    provider: "auto",
    num_results: 5,
    degrade_without_search: true,
  });
  let llm = $state<LlmConfig>({
    provider: "openai",
    model: "gpt-4o-mini",
    base_url: "",
    temperature: 0.8,
    max_tokens: 4096,
  });
  let conv = $state<ConversationConfig | null>(null);
  let ttsModel = $state("openai");
  // 与后端 VideoConfig 的默认值对齐：读盘失败时保留这份默认值，页面不留空白。
  let vid = $state<VideoConfig>({
    aspect: "landscape",
    style: "wave",
    cover: "generated",
    cover_path: "",
    title: "",
    subtitle: "",
    font_path: "",
  });
  let fontStatus = $state("");
  let busy = $state(false);
  let msg = $state("");
  let err = $state("");
  let testing = $state<"llm" | "tts" | "ffmpeg" | "">("");
  // 输出目录配置：dir 为空串 = 用系统默认目录；与后端 OutputConfig 默认值对齐。
  let out = $state<OutputConfig>({ dir: "" });
  let defaultDir = $state("");
  let outputBusy = $state(false);

  async function load() {
    try {
      keys = (await api.getApiKeys()) as typeof keys;
      if (!keys.doubao_resource_id) keys.doubao_resource_id = "volc.service_type.10029";
      if (!keys.doubao_api_version) keys.doubao_api_version = "v1";
      llm = await api.getLlmConfig();
      conv = await api.getConversationConfig();
      ttsModel = conv.text_to_speech.default_tts_model;
      vid = await api.getVideoConfig();
      search = await api.getSearchConfig();
      out = await api.getOutputConfig();
      defaultDir = await api.getDefaultOutputDir();
      await refreshFont(vid.font_path);
    } catch (e) {
      err = String(e);
    }
  }
  onMount(load);

  /** 探明实际会用哪个中文字体：失败不该把整页设置带崩，所以单独 try。 */
  async function refreshFont(path: string) {
    try {
      fontStatus = await api.videoFontStatus(path);
    } catch (e) {
      fontStatus = t("字体不可用：{err}", { err: String(e) });
    }
  }

  async function pickCover() {
    try {
      const p = await open({
        multiple: false,
        filters: [{ name: t("图片"), extensions: ["png", "jpg", "jpeg", "webp"] }],
      });
      if (typeof p === "string") {
        vid.cover_path = p;
        vid.cover = "custom";
      }
    } catch (e) {
      err = String(e);
    }
  }

  async function pickFont() {
    try {
      const p = await open({
        multiple: false,
        filters: [{ name: t("字体"), extensions: ["ttf", "otf", "ttc"] }],
      });
      if (typeof p === "string") {
        vid.font_path = p;
        await refreshFont(p);
      }
    } catch (e) {
      err = String(e);
    }
  }

  async function save() {
    busy = true;
    err = "";
    msg = "";
    try {
      await api.saveApiKeys(keys);
      await api.saveLlmConfig(llm);
      if (conv) {
        conv.text_to_speech.default_tts_model = ttsModel;
        await api.saveConversationConfig(conv);
      }
      if (vid) await api.saveVideoConfig(vid);
      await api.saveSearchConfig(search);
      msg = t("已保存");
    } catch (e) {
      err = String(e);
    } finally {
      busy = false;
    }
  }

  /** 单独保存输出目录：只影响新任务，不动其它配置。 */
  async function saveOutput() {
    outputBusy = true;
    try {
      await api.saveOutputConfig(out);
      toast(t("输出目录已保存"), "ok");
    } catch (e) {
      toast(t("保存输出目录失败：{err}", { err: String(e) }), "err");
    } finally {
      outputBusy = false;
    }
  }

  /** 先落盘再探针：后端读的是已保存配置，否则会测到旧 key。 */
  async function test(kind: "llm" | "tts" | "ffmpeg") {
    testing = kind;
    err = "";
    msg = "";
    await save();
    if (err) {
      testing = "";
      return;
    }
    try {
      msg = await api.testConnection(kind);
    } catch (e) {
      err = String(e);
    } finally {
      testing = "";
    }
  }

  function styleCsv(): string {
    return conv ? conv.conversation_style.join(", ") : "";
  }
  function setStyleCsv(v: string) {
    if (conv) conv.conversation_style = v.split(",").map((s) => s.trim()).filter(Boolean);
  }
  function techCsv(): string {
    return conv ? conv.engagement_techniques.join(", ") : "";
  }
  function setTechCsv(v: string) {
    if (conv) conv.engagement_techniques = v.split(",").map((s) => s.trim()).filter(Boolean);
  }
</script>

<section class="card">
  <h2>{t("设置")}</h2>
  <p class="muted">
    {t("密钥以明文保存在本机应用数据目录，仅靠文件权限保护；除你配置的服务商外不会发往别处，共享电脑请谨慎使用。")}
  </p>

  <div class="form-grid">
    <label>{t("界面语言")}
      <select
        value={getLocale()}
        onchange={(e) => setLocale((e.target as HTMLSelectElement).value as Locale)}
      >
        {#each LOCALES as loc (loc.id)}
          <option value={loc.id}>{loc.label}</option>
        {/each}
      </select>
    </label>
  </div>

  <h3>{t("LLM（转录稿生成）")}</h3>
  <div class="form-grid">
    <label>Provider
      <select bind:value={llm.provider}>
        <option value="openai">{t("openai（兼容任意 OpenAI 接口）")}</option>
        <option value="anthropic">anthropic</option>
        <option value="gemini">gemini</option>
        <option value="ollama">{t("ollama（本地）")}</option>
      </select>
    </label>
    {#if llm.provider === "openai"}
      <label>{t("OpenAI 密钥")}
        <input type="password" bind:value={keys.openai} placeholder="sk-…" />
      </label>
    {/if}
    {#if llm.provider === "anthropic"}
      <label>{t("Anthropic 密钥")}
        <input type="password" bind:value={keys.anthropic} placeholder="sk-ant-…" />
      </label>
    {/if}
    {#if llm.provider === "gemini"}
      <label>{t("Gemini 密钥")}
        <input type="password" bind:value={keys.gemini} placeholder="AIza…" />
      </label>
    {/if}
    {#if llm.provider === "ollama"}
      <p class="muted" style="grid-column: 1 / -1">{t("本地 Ollama 无需密钥。")}</p>
    {/if}
    <label>Model
      <input type="text" bind:value={llm.model} placeholder="gpt-4o-mini" />
    </label>
    <label>{t("Base URL（可选，Ollama 默认 http://localhost:11434/v1）")}
      <input type="text" bind:value={llm.base_url} placeholder="https://api.openai.com/v1" />
    </label>
    <label>Temperature
      <input type="number" step="0.1" min="0" max="2" bind:value={llm.temperature} />
    </label>
    <label>Max tokens
      <input type="number" step="256" min="256" bind:value={llm.max_tokens} />
    </label>
  </div>

  <h3>{t("主题搜索")}</h3>
  <div class="form-grid">
    <label>{t("搜索后端")}
      <select bind:value={search.provider}>
        <option value="auto">{t("auto（按可用性自动选择）")}</option>
        <option value="exa">{t("Exa（免密钥可用）")}</option>
        <option value="serper">Serper（Google）</option>
        <option value="bocha">{t("博查（国内）")}</option>
        <option value="zhipu">{t("智谱（国内）")}</option>
        <option value="qianfan">{t("千帆 / 百度（国内）")}</option>
        <option value="ddg">DuckDuckGo</option>
      </select>
    </label>
    <label>{t("结果条数")}
      <input type="number" min="1" max="20" bind:value={search.num_results} />
    </label>
    <label class="wide">
      <input type="checkbox" bind:checked={search.degrade_without_search} />
      {t("搜索全部失败时，用模型自带知识继续生成（转录稿会标注未经联网检索）")}
    </label>
    <label>{t("Exa 密钥（可空）")}
      <input type="password" bind:value={keys.exa} placeholder={t("不填也能搜，填了提高配额")} />
    </label>
    <label>{t("Serper 密钥（可空）")}
      <input type="password" bind:value={keys.serper} placeholder={t("不填则跳过该后端")} />
    </label>
    <label>{t("博查密钥（可空）")}
      <input type="password" bind:value={keys.bocha} placeholder={t("不填则跳过该后端")} />
    </label>
    <label>{t("智谱密钥（可空）")}
      <input type="password" bind:value={keys.zhipu} placeholder={t("不填则跳过该后端")} />
    </label>
    <label>{t("千帆密钥（可空）")}
      <input type="password" bind:value={keys.qianfan} placeholder={t("不填则跳过该后端")} />
    </label>
  </div>

  <h3>{t("其它（可选）")}</h3>
  <div class="form-grid">
    <label>{t("ElevenLabs（预留）")}
      <input type="password" bind:value={keys.elevenlabs} placeholder={t("预留")} disabled />
    </label>
  </div>

  {#if conv}
    <h3>{t("播客人设")}</h3>
    <div class="form-grid">
      <label>{t("播客名")}
        <input type="text" bind:value={conv.podcast_name} />
      </label>
      <label>Slogan
        <input type="text" bind:value={conv.podcast_tagline} />
      </label>
      <label>{t("输出语言")}
        <input type="text" list="output-language-options" bind:value={conv.output_language} placeholder={t("English / 中文")} />
        <datalist id="output-language-options">
          <option value="English"></option>
          <option value="简体中文">{t("简体中文")}</option>
          <option value="繁體中文">{t("繁體中文")}</option>
        </datalist>
      </label>
      <label>{t("主持人 1 角色")}
        <input type="text" bind:value={conv.roles_person1} />
      </label>
      <label>{t("主持人 2 角色")}
        <input type="text" bind:value={conv.roles_person2} />
      </label>
      <label>{t("风格（逗号分隔）")}
        <input type="text" value={styleCsv()} oninput={(e) => setStyleCsv((e.target as HTMLInputElement).value)} />
      </label>
      <label>{t("互动技巧（逗号分隔）")}
        <input type="text" value={techCsv()} oninput={(e) => setTechCsv((e.target as HTMLInputElement).value)} />
      </label>
      <label>{t("创造力 0–2")}
        <input type="number" step="0.1" min="0" max="2" bind:value={conv.creativity} />
      </label>
      <label>{t("长片最大轮数")}
        <input type="number" min="1" max="40" bind:value={conv.max_num_chunks} />
      </label>
      <label class="wide">{t("自定义指令")}
        <textarea rows="2" bind:value={conv.user_instructions} placeholder={t("可选，追加给 LLM 的指令")}></textarea>
      </label>
    </div>

    <h3>{t("语音（TTS）")}</h3>
    <div class="form-grid">
      <label>{t("默认 TTS")}
        <select bind:value={ttsModel}>
          <option value="openai">openai</option>
          <option value="edge">{t("edge（免费）")}</option>
          <option value="doubao">{t("豆包（火山引擎）")}</option>
        </select>
      </label>
      {#if ttsModel === "openai"}
        <label>{t("OpenAI 主持人 1 音色")}
          <input type="text" bind:value={conv.text_to_speech.openai.question} />
        </label>
        <label>{t("OpenAI 主持人 2 音色")}
          <input type="text" bind:value={conv.text_to_speech.openai.answer} />
        </label>
      {/if}
      {#if ttsModel === "edge"}
        <label>{t("Edge 主持人 1")}
          <input type="text" bind:value={conv.text_to_speech.edge.question} />
        </label>
        <label>{t("Edge 主持人 2")}
          <input type="text" bind:value={conv.text_to_speech.edge.answer} />
        </label>
      {/if}
      {#if ttsModel === "doubao"}
        <p class="muted" style="grid-column: 1 / -1">{t("仅在选用豆包 TTS 时需要填写以下豆包凭证。")}</p>
        <label>{t("豆包 App ID")}
          <input type="text" bind:value={keys.doubao_app_id} placeholder="app-id" />
        </label>
        <label>{t("豆包 Access Token")}
          <input type="password" bind:value={keys.doubao_access_token} placeholder="access-token" />
        </label>
        <label>{t("豆包 Resource ID")}
          <input type="text" bind:value={keys.doubao_resource_id} placeholder="volc.service_type.10029" />
        </label>
        <label>{t("豆包 API 版本")}
          <select bind:value={keys.doubao_api_version}>
            <option value="v1">v1（ws_binary）</option>
            <option value="v3">v3（bidirection）</option>
          </select>
        </label>
        <label>{t("豆包集群")}
          <input type="text" bind:value={conv.text_to_speech.doubao.model} placeholder="volcano_tts" />
        </label>
        <label>{t("豆包主持人 1 音色")}
          <input type="text" bind:value={conv.text_to_speech.doubao.question} />
        </label>
        <label>{t("豆包主持人 2 音色")}
          <input type="text" bind:value={conv.text_to_speech.doubao.answer} />
        </label>
      {/if}
    </div>
  {/if}

  <h3>{t("视频导出")}</h3>
  <div class="form-grid">
    <label>{t("画幅")}
      <select bind:value={vid.aspect}>
        <option value="landscape">{t("横版 16:9")}</option>
        <option value="portrait">{t("竖版 9:16（短视频）")}</option>
      </select>
    </label>
    <label>{t("画面风格")}
      <select bind:value={vid.style}>
        <option value="wave">{t("波形（推荐）")}</option>
        <option value="cover">{t("静态封面")}</option>
      </select>
    </label>
    {#if vid.style === "cover"}
      <label>{t("封面来源")}
        <select bind:value={vid.cover}>
          <option value="generated">{t("自动生成渐变底图")}</option>
          <option value="custom">{t("自选图片")}</option>
        </select>
      </label>
      {#if vid.cover === "custom"}
        <label>{t("封面图片")}
          <input type="text" bind:value={vid.cover_path} placeholder="/path/to/cover.png" />
        </label>
      {/if}
    {/if}
    <label>{t("标题")}
      <input type="text" bind:value={vid.title} placeholder={t("留空则使用任务标题")} />
    </label>
    <label>{t("副标题")}
      <input type="text" bind:value={vid.subtitle} placeholder={t("留空则使用播客标语")} />
    </label>
    <label>{t("自定义中文字体（可选）")}
      <input
        type="text"
        bind:value={vid.font_path}
        placeholder={t("留空则使用随包字体")}
        onchange={(e) => refreshFont((e.currentTarget as HTMLInputElement).value)}
      />
    </label>
  </div>
  <div class="actions">
    {#if vid.style === "cover" && vid.cover === "custom"}
      <button class="btn" type="button" onclick={pickCover}>{t("选择图片…")}</button>
    {/if}
    <button class="btn" type="button" onclick={pickFont}>{t("选择字体文件…")}</button>
  </div>
  <p class="muted">{t("当前中文字体：{path}", { path: fontStatus })}</p>
  <p class="muted">
    {t("视频由 ffmpeg 编码，导出在任务卡片上手动触发，不会随生成自动执行。")}
  </p>

  <h3>{t("输出目录")}</h3>
  <div class="form-grid">
    <label>{t("自定义输出目录")}
      <input type="text" bind:value={out.dir} placeholder={t("留空则使用系统默认目录")} />
    </label>
  </div>
  <div class="actions">
    <button class="btn" type="button" onclick={saveOutput} disabled={outputBusy}>
      {outputBusy ? t("保存中…") : t("保存输出目录")}
    </button>
  </div>
  <p class="muted">{t("系统默认目录：{path}", { path: defaultDir })}</p>
  <p class="muted">
    {t("留空则使用系统默认目录（Linux ~/.local/share、Windows %APPDATA%、macOS ~/Library/Application Support，跨平台自动适配）。修改后只对新任务生效，已有任务仍留在原目录。")}
  </p>

  {#if msg}<p class="ok">{msg}</p>{/if}
  {#if err}<p class="err">{err}</p>{/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={save} disabled={busy}>
      {busy ? t("保存中…") : t("保存设置")}
    </button>
    <button class="btn" onclick={load}>{t("重新加载")}</button>
  </div>

  <div class="actions probe">
    <span class="probe-label">{t("连通性自检")}</span>
    <button class="btn" onclick={() => test("llm")} disabled={testing !== "" || busy}>
      {testing === "llm" ? t("测试中…") : t("测试 LLM")}
    </button>
    <button class="btn" onclick={() => test("tts")} disabled={testing !== "" || busy}>
      {testing === "tts" ? t("测试中…") : t("测试 TTS")}
    </button>
    <button class="btn" onclick={() => test("ffmpeg")} disabled={testing !== "" || busy}>
      {testing === "ffmpeg" ? t("测试中…") : t("测试 FFmpeg")}
    </button>
    <span class="muted">{t("走真实请求路径，仅消耗一次极小额度")}</span>
  </div>
</section>

<style>
  .actions {
    display: flex;
    gap: var(--sp-2);
    margin-top: var(--sp-4);
  }
  .actions.probe {
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-2) var(--sp-3);
    margin-top: var(--sp-3);
    padding-top: var(--sp-3);
    border-top: 1px solid var(--border);
  }
  .probe-label {
    font-size: var(--fs-sm, 0.85rem);
    color: var(--fg-subtle);
  }
</style>
