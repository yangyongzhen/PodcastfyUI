<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import type { ApiKeys, Task } from "../lib/types";
  import NewTask from "./NewTask.svelte";
  import TaskCard from "./TaskCard.svelte";
  import Settings from "./Settings.svelte";
  import { toast } from "../lib/ui/feedback.svelte.js";
  import { t } from "../lib/i18n.svelte.js";

  let tasks = $state<Task[]>([]);
  let view = $state<"new" | "list" | "settings">("new");
  let needsSetup = $state(false);
  let unlisten: (() => void) | null = null;

  let lastErrText = "";
  let lastErrAt = 0;

  async function refresh() {
    try {
      tasks = await api.listTasks();
    } catch (e) {
      // 轮询 5s 一次，同一个错误 30s 内只提示一次，避免刷屏。
      const text = String(e);
      const now = Date.now();
      if (text !== lastErrText || now - lastErrAt > 30_000) {
        lastErrText = text;
        lastErrAt = now;
        toast(t("任务列表加载失败：{err}", { err: text }), "err");
      }
      console.error("list_tasks failed", e);
    }
  }

  function providerKey(keys: ApiKeys, provider: string): string {
    switch (provider) {
      case "openai":
        return keys.openai;
      case "anthropic":
        return keys.anthropic;
      case "gemini":
        return keys.gemini;
      default:
        return ""; // ollama 等本地模型不需要 key
    }
  }

  /** 首次运行引导：缺 key 时提示去设置，而不是让用户提交后才发现跑不起来。 */
  async function checkSetup() {
    try {
      const [keys, llm, conv] = await Promise.all([
        api.getApiKeys(),
        api.getLlmConfig(),
        api.getConversationConfig(),
      ]);
      const needsLlmKey = llm.provider !== "ollama" && !providerKey(keys, llm.provider);
      const needsTtsKey =
        conv.text_to_speech.default_tts_model === "openai" && !keys.openai;
      needsSetup = needsLlmKey || needsTtsKey;
    } catch (e) {
      // 读取失败不阻断主流程，列表错误会有 toast 提示。
      console.error("check_setup failed", e);
    }
  }

  onMount(() => {
    refresh();
    // Live updates: any task-update event triggers a refresh (polling below
    // is the safety net for missed events).
    api
      .onTaskUpdate(() => refresh())
      .then((u) => (unlisten = u));
    // Poll as a safety net (events can be missed if the window was hidden).
    const timer = setInterval(refresh, 5000);
    return () => {
      unlisten?.();
      clearInterval(timer);
    };
  });

  // 从设置页返回时重新判断是否需要引导（刚填完 key 就该消失）。
  $effect(() => {
    if (view !== "settings") checkSetup();
  });

  const RUNNING_STATUS = ["pending", "extracting", "generating", "synthesizing", "muxing"];
  const FINISHED_STATUS = ["completed", "failed", "cancelled"];

  type Filter = "all" | "running" | "completed" | "failed";
  let filter = $state<Filter>("all");
  let query = $state("");

  // 导航栏徽标用全量统计，不受搜索/筛选影响。
  const running = $derived(tasks.filter((task) => RUNNING_STATUS.includes(task.status)));
  const countCompleted = $derived(tasks.filter((task) => task.status === "completed").length);
  const countFailed = $derived(
    tasks.filter((task) => task.status === "failed" || task.status === "cancelled").length,
  );

  /** 搜索 + 状态筛选后可见的任务。 */
  const visible = $derived(
    tasks.filter((task) => {
      const q = query.trim().toLowerCase();
      if (q && !task.title.toLowerCase().includes(q)) return false;
      switch (filter) {
        case "running":
          return RUNNING_STATUS.includes(task.status);
        case "completed":
          return task.status === "completed";
        case "failed":
          return task.status === "failed" || task.status === "cancelled";
        default:
          return true;
      }
    }),
  );
  const visibleRunning = $derived(visible.filter((task) => RUNNING_STATUS.includes(task.status)));
  const visibleDone = $derived(visible.filter((task) => FINISHED_STATUS.includes(task.status)));
</script>

<div class="app">
  <header class="topbar">
    <div class="brand">🎙️ Podcastfy <span class="sub">Rust · Tauri</span></div>
    <nav>
      <button class="btn" class:active={view === "new"} onclick={() => (view = "new")}>
        {t("新建")}
      </button>
      <button class="btn" class:active={view === "list"} onclick={() => { view = "list"; refresh(); }}>
        {t("任务")}{running.length > 0 ? t("（{n} 进行中）", { n: running.length }) : ""}
      </button>
      <button class="btn" class:active={view === "settings"} onclick={() => (view = "settings")}>
        {t("设置")}
      </button>
    </nav>
  </header>

  {#if needsSetup && view !== "settings"}
    <div class="banner">
      <span>{t("还没有可用的 API 密钥，现在提交任务会在生成阶段失败。")}</span>
      <button class="btn btn-primary" onclick={() => (view = "settings")}>{t("去设置")}</button>
    </div>
  {/if}

  <main>
    {#if view === "new"}
      <div class="two">
        <NewTask onStarted={() => (view = "list")} />
        <aside class="card tip">
          <h3>{t("使用说明")}</h3>
          <ol>
            <li>{t("先在「设置」里填好 API 密钥（LLM 与 TTS 至少各一个；TTS 选 edge 则免 key）")}</li>
            <li>{t("回到「新建」，粘贴网页 / YouTube 链接，或填主题 / 纯文本 / PDF")}</li>
            <li>{t("点「生成播客」，到「任务」页看实时进度")}</li>
            <li>{t("完成后可直接播放、编辑转录稿、删除任务")}</li>
          </ol>
          <p class="note">{t("需要系统安装")} <code>ffmpeg</code>{t("（音频拼接用）。")}</p>
        </aside>
      </div>
    {:else if view === "list"}
      <div class="toolbar">
        <input
          class="search"
          type="text"
          placeholder={t("搜索标题…")}
          bind:value={query}
          aria-label={t("按标题搜索任务")}
        />
        <div class="filters" role="group" aria-label={t("按状态筛选")}>
          <button class="chip" class:on={filter === "all"} onclick={() => (filter = "all")}>
            {t("全部")} {tasks.length}
          </button>
          <button class="chip" class:on={filter === "running"} onclick={() => (filter = "running")}>
            {t("进行中")} {running.length}
          </button>
          <button
            class="chip"
            class:on={filter === "completed"}
            onclick={() => (filter = "completed")}
          >
            {t("已完成")} {countCompleted}
          </button>
          <button class="chip" class:on={filter === "failed"} onclick={() => (filter = "failed")}>
            {t("失败")} {countFailed}
          </button>
        </div>
      </div>
      {#if filter === "all"}
        {#if visibleRunning.length}
          <h2 class="h2">{t("进行中")}</h2>
          <div class="cards">
            {#each visibleRunning as task (task.id)}
              <TaskCard task={task} onRefresh={refresh} />
            {/each}
          </div>
        {/if}
        {#if visibleDone.length}
          <h2 class="h2">{t("历史")}</h2>
          <div class="cards">
            {#each visibleDone as task (task.id)}
              <TaskCard task={task} onRefresh={refresh} />
            {/each}
          </div>
        {/if}
      {:else}
        <h2 class="h2">
          {filter === "running" ? t("进行中") : filter === "completed" ? t("已完成") : t("失败 / 已取消")}
        </h2>
        <div class="cards">
          {#each visible as task (task.id)}
            <TaskCard task={task} onRefresh={refresh} />
          {/each}
        </div>
      {/if}
      {#if visible.length === 0}
        {#if tasks.length === 0}
          <div class="empty-state">
            <svg class="empty-art" viewBox="0 0 160 120" aria-hidden="true">
              <defs>
                <linearGradient id="emptyBrand" x1="0" y1="0" x2="1" y2="1">
                  <stop offset="0" style="stop-color: var(--brand-1)" />
                  <stop offset="1" style="stop-color: var(--brand-2)" />
                </linearGradient>
              </defs>
              <rect x="70" y="26" width="20" height="36" rx="10" fill="url(#emptyBrand)" />
              <path
                d="M62 58a18 18 0 0 0 36 0"
                fill="none"
                style="stroke: var(--brand)"
                stroke-width="3"
                stroke-linecap="round"
              />
              <path
                d="M80 76v10M68 90h24"
                fill="none"
                style="stroke: var(--brand)"
                stroke-width="3"
                stroke-linecap="round"
              />
              <path
                d="M44 44c-6 8-6 24 0 32"
                fill="none"
                style="stroke: var(--brand-2)"
                stroke-width="3"
                stroke-linecap="round"
                opacity="0.7"
              />
              <path
                d="M30 36c-10 12-10 36 0 48"
                fill="none"
                style="stroke: var(--brand-2)"
                stroke-width="3"
                stroke-linecap="round"
                opacity="0.4"
              />
              <path
                d="M116 44c6 8 6 24 0 32"
                fill="none"
                style="stroke: var(--brand-2)"
                stroke-width="3"
                stroke-linecap="round"
                opacity="0.7"
              />
              <path
                d="M130 36c10 12 10 36 0 48"
                fill="none"
                style="stroke: var(--brand-2)"
                stroke-width="3"
                stroke-linecap="round"
                opacity="0.4"
              />
            </svg>
            <h3>{t("还没有播客任务")}</h3>
            <p class="muted">
              {t("粘贴网页 / YouTube 链接、选一份 PDF，或直接输入主题，就能生成双人对话播客。")}
            </p>
            <a class="btn btn-primary" href="#new-task">{t("开始创建第一个播客")}</a>
          </div>
        {:else}
          <p class="empty">{t("没有匹配的任务（试试清空搜索或换筛选条件）。")}</p>
        {/if}
      {/if}
    {:else}
      <Settings />
    {/if}
  </main>
</div>

<style>
  .app {
    max-width: 1180px;
    margin: 0 auto;
    padding: 18px 24px 28px;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .topbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--sp-4);
    padding: 6px 2px 18px;
  }
  .brand {
    font-size: var(--fs-xl);
    font-weight: 700;
    letter-spacing: -0.01em;
    white-space: nowrap;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--fg-subtle);
    font-weight: 400;
    margin-left: var(--sp-2);
  }
  nav {
    display: flex;
    gap: var(--sp-2);
  }
  nav button.active {
    background: var(--brand);
    border-color: var(--brand);
    color: var(--brand-contrast);
    box-shadow: var(--shadow-1);
  }
  nav button.active:hover {
    background: var(--brand-strong);
    border-color: var(--brand-strong);
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    margin-bottom: var(--sp-4);
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--brand);
    border-radius: var(--r-md);
    background: var(--info-bg);
    color: var(--info-fg);
    font-size: var(--fs-md);
  }
  main {
    flex: 1;
  }
  .two {
    display: grid;
    grid-template-columns: 3fr 2fr;
    gap: var(--sp-4);
    align-items: start;
  }
  @media (max-width: 900px) {
    .two {
      grid-template-columns: 1fr;
    }
  }
  .tip {
    padding: var(--sp-4);
  }
  .tip h3 {
    margin: 0 0 var(--sp-2);
    font-size: var(--fs-md);
  }
  .tip ol {
    padding-left: 18px;
    margin: var(--sp-2) 0;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .tip li {
    margin-bottom: var(--sp-1);
  }
  .note {
    font-size: var(--fs-sm);
    color: var(--fg-subtle);
    margin: 0;
  }
  code {
    background: var(--bg-inset);
    border-radius: var(--r-sm);
    padding: 1px 5px;
    font-size: 0.92em;
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .h2 {
    font-size: var(--fs-md);
    font-weight: 600;
    color: var(--fg-muted);
    margin: var(--sp-3) 0 var(--sp-2);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    margin-bottom: var(--sp-3);
    flex-wrap: wrap;
  }
  .search {
    flex: 1 1 200px;
    min-width: 0;
  }
  .filters {
    display: flex;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }
  .chip {
    border: 1px solid var(--border);
    border-radius: var(--r-full);
    background: var(--bg-elev);
    color: var(--fg-muted);
    font: inherit;
    font-size: var(--fs-sm);
    padding: 5px 12px;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s, color 0.15s;
  }
  .chip:hover {
    border-color: var(--border-strong);
    color: var(--fg);
  }
  .chip.on {
    background: var(--brand);
    border-color: var(--brand);
    color: var(--brand-contrast);
    font-weight: 600;
  }
  .chip:focus-visible {
    outline: 2px solid var(--brand);
    outline-offset: 2px;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-5) var(--sp-4);
    margin-top: var(--sp-3);
    border: 1px dashed var(--border);
    border-radius: var(--r-md, 8px);
    background: var(--bg-inset);
    text-align: center;
  }
  .empty-art {
    width: 160px;
    height: 120px;
    max-width: 60%;
  }
  .empty-state h3 {
    margin: 0;
    font-size: var(--fs-lg, 1.05rem);
  }
  .empty-state p {
    margin: 0;
    max-width: 42ch;
    line-height: 1.6;
  }
  .empty {
    color: var(--fg-subtle);
    font-size: var(--fs-md);
    text-align: center;
    margin-top: 64px;
  }
</style>
