<script lang="ts">
  import * as api from "../lib/api";
  import type { TaskInput } from "../lib/types";
  import { open } from "@tauri-apps/plugin-dialog";
  import { toast } from "../lib/ui/feedback.svelte.js";
  import { t } from "../lib/i18n.svelte.js";

  const { onStarted }: { onStarted: (id: string) => void } = $props();

  let urls = $state("");
  let pdfs = $state<string[]>([]);
  let text = $state("");
  let topic = $state("");
  let longform = $state(false);
  let busy = $state(false);
  let error = $state("");

  function buildInput(): TaskInput {
    return {
      urls: urls
        .split("\n")
        .map((s) => s.trim())
        .filter(Boolean),
      pdfs: pdfs.map((s) => s.trim()).filter(Boolean),
      text: text.trim(),
      topic: topic.trim(),
      longform,
    };
  }

  function hasInput(): boolean {
    const i = buildInput();
    return (
      i.urls.length > 0 ||
      i.pdfs.length > 0 ||
      i.text.length > 0 ||
      i.topic.length > 0
    );
  }

  /** 走系统文件选择器，避免让用户手抄绝对路径。 */
  async function pickPdfs() {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: "PDF", extensions: ["pdf"] }],
      });
      if (!selected) return;
      const list = Array.isArray(selected) ? selected : [selected];
      for (const p of list) {
        if (!pdfs.includes(p)) pdfs.push(p);
      }
    } catch (e) {
      error = t("选择文件失败：{err}", { err: String(e) });
    }
  }

  function removePdf(path: string) {
    pdfs = pdfs.filter((p) => p !== path);
  }

  async function start() {
    if (!hasInput()) {
      error = t("请至少提供 URL、PDF、文本或主题之一");
      return;
    }
    error = "";
    busy = true;
    try {
      const title = buildInput().topic || "Podcast";
      const id = await api.startTask(title, buildInput());
      onStarted(id);
      urls = "";
      pdfs = [];
      text = "";
      topic = "";
      longform = false;
      toast(t("任务已创建"), "ok");
    } catch (e) {
      error = String(e);
      toast(t("创建任务失败：{err}", { err: String(e) }), "err");
    } finally {
      busy = false;
    }
  }
</script>

<section class="card" id="new-task">
  <h2>{t("新建播客")}</h2>

  <label class="field">
    <span>{t("网页 / YouTube 链接（每行一个）")}</span>
    <textarea rows="3" bind:value={urls} placeholder="https://…&#10;https://youtu.be/…"></textarea>
  </label>

  <div class="field">
    <span>{t("本地 PDF（可选）")}</span>
    <div class="pdf-row">
      <button class="btn" type="button" onclick={pickPdfs}>{t("选择 PDF 文件…")}</button>
      <span class="muted">{t("可多选")}</span>
    </div>
    {#if pdfs.length}
      <ul class="pdf-list">
        {#each pdfs as p (p)}
          <li>
            <span class="path" title={p}>{p}</span>
            <button
              class="rm"
              type="button"
              onclick={() => removePdf(p)}
              aria-label={t("移除 {p}", { p })}
            >
              ×
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <label class="field">
    <span>{t("纯文本（可选）")}</span>
    <textarea rows="3" bind:value={text} placeholder={t("粘贴文章内容…")}></textarea>
  </label>

  <label class="field">
    <span>{t("主题（联网搜索扩展，可选）")}</span>
    <input type="text" bind:value={topic} placeholder={t("如：量子计算的最新进展")} />
  </label>

  <label class="check">
    <input type="checkbox" bind:checked={longform} />
    {t("长篇模式（30 分钟+，多轮讨论）")}
  </label>

  {#if error}
    <p class="err">{error}</p>
  {/if}

  <button class="btn btn-primary btn-block" onclick={start} disabled={busy}>
    {busy ? t("启动中…") : t("🎙️ 生成播客")}
  </button>
</section>

<style>
  .check {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-bottom: var(--sp-3);
    font-size: var(--fs-md);
    color: var(--fg-muted);
    cursor: pointer;
  }
  .check input {
    width: auto;
    margin: 0;
    accent-color: var(--brand);
  }
  .pdf-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }
  .pdf-list {
    list-style: none;
    margin: var(--sp-2) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }
  .pdf-list li {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    background: var(--bg-inset);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
  }
  .rm {
    flex: 0 0 auto;
    border: none;
    background: none;
    color: var(--fg-subtle);
    font-size: 1rem;
    line-height: 1;
    padding: 0 4px;
    cursor: pointer;
  }
  .rm:hover {
    color: var(--err-fg);
  }
</style>
