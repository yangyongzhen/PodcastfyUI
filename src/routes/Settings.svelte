<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import type { ConversationConfig, LlmConfig } from "../lib/types";
  import { t, LOCALES, getLocale, setLocale, type Locale } from "../lib/i18n.svelte.js";

  let keys = $state({ openai: "", anthropic: "", gemini: "", elevenlabs: "", serper: "", doubao_app_id: "", doubao_access_token: "", doubao_resource_id: "volc.service_type.10029", doubao_api_version: "v1" });
  let llm = $state<LlmConfig>({
    provider: "openai",
    model: "gpt-4o-mini",
    base_url: "",
    temperature: 0.8,
    max_tokens: 4096,
  });
  let conv = $state<ConversationConfig | null>(null);
  let ttsModel = $state("openai");
  let busy = $state(false);
  let msg = $state("");
  let err = $state("");
  let testing = $state<"llm" | "tts" | "ffmpeg" | "">("");

  async function load() {
    try {
      keys = (await api.getApiKeys()) as typeof keys;
      if (!keys.doubao_resource_id) keys.doubao_resource_id = "volc.service_type.10029";
      if (!keys.doubao_api_version) keys.doubao_api_version = "v1";
      llm = await api.getLlmConfig();
      conv = await api.getConversationConfig();
      ttsModel = conv.text_to_speech.default_tts_model;
    } catch (e) {
      err = String(e);
    }
  }
  onMount(load);

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
      msg = t("已保存");
    } catch (e) {
      err = String(e);
    } finally {
      busy = false;
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

  <h3>{t("其它（可选）")}</h3>
  <div class="form-grid">
    <label>{t("ElevenLabs（预留）")}
      <input type="password" bind:value={keys.elevenlabs} placeholder={t("预留")} disabled />
    </label>
    <label>{t("Serper（主题搜索，可选）")}
      <input type="password" bind:value={keys.serper} placeholder={t("不填则用 DuckDuckGo")} />
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
