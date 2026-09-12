<script lang="ts">
  import { untrack } from "svelte";
  import * as api from "../lib/api";
  import type { Task, TaskStatus } from "../lib/types";
  import { confirmDialog, toast } from "../lib/ui/feedback.svelte.js";
  import { parseTranscript } from "../lib/transcript";
  import { t } from "../lib/i18n.svelte.js";

  const { task, onRefresh }: { task: Task; onRefresh: () => void } = $props();

  let showTranscript = $state(false);
  let transcript = $state("");
  let editing = $state(false);
  let editBuf = $state("");
  let saving = $state(false);

  function statusLabel(s: TaskStatus): string {
    switch (s) {
      case "pending":
        return t("排队中");
      case "extracting":
        return t("抽取内容");
      case "generating":
        return t("生成对话稿");
      case "synthesizing":
        return t("语音合成");
      case "muxing":
        return t("音频合成");
      case "exporting":
        return t("导出视频");
      case "completed":
        return t("已完成");
      case "failed":
        return t("失败");
      default:
        return t("已取消");
    }
  }

  const isRunning = (): boolean =>
    !["completed", "failed", "cancelled"].includes(task.status);

  // ---- B 级：进度可访问性 + 剩余时间预估（线性外推） --------------------
  // 计时从「组件首次看到运行态」算起，而不是 created_at —— 否则重新合成
  // 音频时（任务已创建很久）预估会离谱。
  let nowTick = $state(0);
  let runStartedAt = $state<number | null>(null);

  $effect(() => {
    const running = isRunning();
    if (!running) {
      untrack(() => {
        runStartedAt = null;
      });
      return;
    }
    if (untrack(() => runStartedAt) === null) runStartedAt = Date.now();
  });

  $effect(() => {
    if (!isRunning()) return;
    const timer = setInterval(() => nowTick++, 1000);
    return () => clearInterval(timer);
  });

  const etaText = $derived.by(() => {
    void nowTick; // 每秒重算，否则预估会停住不动
    const p = task.progress;
    if (!isRunning() || runStartedAt === null || p < 8 || p >= 100) return "";
    const elapsed = Math.max(0, Date.now() - runStartedAt);
    return fmtDuration((elapsed * (100 - p)) / p);
  });

  function fmtDuration(ms: number): string {
    const s = Math.round(ms / 1000);
    if (s < 60) return t("{n} 秒", { n: Math.max(1, s) });
    const m = Math.round(s / 60);
    if (m < 60) return t("{n} 分钟", { n: m });
    return t("{h} 小时 {m} 分", { h: Math.floor(m / 60), m: m % 60 });
  }

  const view = $derived(parseTranscript(transcript));

  async function toggleTranscript() {
    showTranscript = !showTranscript;
    if (showTranscript && !transcript) {
      try {
        transcript = (await api.getTranscript(task.id)) ?? "";
      } catch (e) {
        transcript = "";
        toast(t("转录稿加载失败：{err}", { err: String(e) }), "err");
      }
    }
  }

  function startEdit() {
    editBuf = transcript;
    editing = true;
  }

  async function saveEdit() {
    saving = true;
    try {
      await api.saveTranscript(task.id, editBuf);
      transcript = editBuf;
      editing = false;
      toast(t("转录稿已保存"), "ok");
    } catch (e) {
      toast(t("保存失败：{err}", { err: String(e) }), "err");
    } finally {
      saving = false;
    }
  }

  let resynthesizing = $state(false);

  /** 改完转录稿后只重跑 TTS + 拼接，不必重新调用 LLM。 */
  async function resynthesize() {
    const ok = await confirmDialog(
      t("将用当前转录稿重新合成音频，并覆盖已有音频文件。继续吗？"),
      { title: t("仅重新合成音频"), confirmText: t("开始合成") },
    );
    if (!ok) return;
    resynthesizing = true;
    try {
      await api.resynthesizeTask(task.id);
      toast(t("已开始重新合成，进度见本卡片"));
      onRefresh();
    } catch (e) {
      toast(t("重新合成失败：{err}", { err: String(e) }), "err");
    } finally {
      resynthesizing = false;
    }
  }

  async function cancel() {
    try {
      await api.cancelTask(task.id);
      toast(t("已请求取消该任务"));
    } catch (e) {
      toast(String(e), "err");
    }
    onRefresh();
  }

  async function remove() {
    const ok = await confirmDialog(
      t("删除任务「{title}」？该任务的转录稿与音频文件会一并删除，且不可恢复。", {
        title: task.title,
      }),
      { title: t("删除任务"), confirmText: t("删除"), danger: true },
    );
    if (!ok) return;
    try {
      await api.deleteTask(task.id);
      toast(t("任务已删除"), "ok");
    } catch (e) {
      toast(t("删除失败：{err}", { err: String(e) }), "err");
    }
    onRefresh();
  }

  async function openAudio() {
    try {
      await api.openAudioFile(task.id);
    } catch (e) {
      toast(String(e), "err");
    }
  }

  const audioUrl = $derived(task.audio_path ? api.fileToAssetUrl(task.audio_path) : null);

  // ---- 视频导出（可选第五阶段：L1 封面 / L2 波形） --------------------
  let exporting = $state(false);
  const videoUrl = $derived(task.video_path ? api.fileToAssetUrl(task.video_path) : null);

  async function exportVideo() {
    exporting = true;
    try {
      await api.exportVideo(task.id);
      toast(t("视频已导出"));
      onRefresh();
    } catch (e) {
      // 后端把失败原因记在 video_error 上，音频产物不受影响；刷新后卡片会显示出来。
      toast(String(e), "err");
      onRefresh();
    } finally {
      exporting = false;
    }
  }

  async function openVideo() {
    try {
      await api.openVideoFile(task.id);
    } catch (e) {
      toast(String(e), "err");
    }
  }

  // ---- 自定义音频播放器 ----
  const rates = [1, 1.25, 1.5, 2] as const;
  let audioEl = $state<HTMLAudioElement | null>(null);
  let playing = $state(false);
  let duration = $state(0);
  let currentTime = $state(0);
  let rateIdx = $state(0);
  const rate = $derived(rates[rateIdx]);

  function togglePlay() {
    if (!audioEl) return;
    if (audioEl.paused) void audioEl.play();
    else audioEl.pause();
  }

  function seekBy(sec: number) {
    if (!audioEl) return;
    const d = Number.isFinite(audioEl.duration) ? audioEl.duration : 0;
    currentTime = Math.min(Math.max(audioEl.currentTime + sec, 0), d);
    audioEl.currentTime = currentTime;
  }

  function onSeek(e: Event) {
    if (!audioEl) return;
    const v = Number((e.currentTarget as HTMLInputElement).value);
    currentTime = v;
    audioEl.currentTime = v;
  }

  function onTimeUpdate(e: Event) {
    currentTime = (e.currentTarget as HTMLAudioElement).currentTime;
  }

  function onLoaded(e: Event) {
    const el = e.currentTarget as HTMLAudioElement;
    duration = el.duration;
    el.playbackRate = rates[rateIdx];
  }

  function cycleRate() {
    rateIdx = (rateIdx + 1) % rates.length;
    if (audioEl) audioEl.playbackRate = rates[rateIdx];
  }

  function fmtClock(sec: number): string {
    if (!Number.isFinite(sec) || sec <= 0) return "0:00";
    const total = Math.floor(sec);
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  }

  function fmtTime(s: string): string {
    // Backend sends unix-seconds string in the skeleton; fall back gracefully.
    const n = Number(s);
    if (!Number.isFinite(n) || n < 10_000) return s;
    return new Date(n * 1000).toLocaleString();
  }
</script>

<article class="card task">
  <header>
    <div class="title">
      <strong>{task.title}</strong>
      <span class="badge {task.status}">{statusLabel(task.status)}</span>
    </div>
    <div class="meta">{fmtTime(task.created_at)}</div>
  </header>

  {#if isRunning()}
    <div
      class="progress"
      role="progressbar"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={task.progress}
      aria-valuetext="{task.progress}% · {task.stage}"
      aria-label={t("生成进度")}
    >
      <div class="progress-fill" style="width: {task.progress}%"></div>
    </div>
    <p class="stage">
      <span role="status" aria-live="polite">{task.stage}</span>（{task.progress}%）{#if etaText}<span
          class="eta">{t("预计还需约 {eta}", { eta: etaText })}</span>{/if}
    </p>
    <div class="actions">
      <button class="btn" onclick={cancel}>{t("取消")}</button>
    </div>
  {:else}
    {#if task.status === "completed"}
      {#if audioUrl}
        <div class="player">
          <audio
            class="audio"
            bind:this={audioEl}
            src={audioUrl}
            onplay={() => (playing = true)}
            onpause={() => (playing = false)}
            onended={() => (playing = false)}
            ontimeupdate={onTimeUpdate}
            onloadedmetadata={onLoaded}
          ></audio>
          <div class="controls">
            <button
              class="ctl"
              type="button"
              aria-label={playing ? t("暂停") : t("播放")}
              onclick={togglePlay}
            >
              {playing ? "⏸" : "▶"}
            </button>
            <button
              class="ctl"
              type="button"
              aria-label={t("后退10秒")}
              onclick={() => seekBy(-10)}
            >
              −10s
            </button>
            <button
              class="ctl"
              type="button"
              aria-label={t("前进10秒")}
              onclick={() => seekBy(10)}
            >
              +10s
            </button>
            <input
              class="seek"
              type="range"
              min="0"
              max={duration}
              step="0.1"
              value={currentTime}
              aria-label={t("播放进度")}
              oninput={onSeek}
            />
            <span class="time">{fmtClock(currentTime)} / {fmtClock(duration)}</span>
            <button
              class="ctl rate"
              class:active={rate !== 1}
              type="button"
              aria-label={t("倍速，当前 {rate}×，点击切换", { rate })}
              title={t("播放速度")}
              onclick={cycleRate}
            >
              {rate}×
            </button>
          </div>
        </div>
      {/if}

      {#if videoUrl}
        <div class="player">
          <!-- 成片没有独立字幕轨：标题/副标题已烧进画面，故显式忽略该 a11y 提示 -->
          <!-- svelte-ignore a11y_media_has_caption -->
          <video class="video" src={videoUrl} controls preload="metadata"></video>
        </div>
      {/if}
      {#if task.video_error}
        <p class="warn">{t("视频导出失败：{err}", { err: task.video_error })}</p>
      {/if}

      <div class="actions">
        <button class="btn" onclick={toggleTranscript}>
          {showTranscript ? t("收起转录稿") : t("查看/编辑转录稿")}
        </button>
        <button class="btn" onclick={openAudio}>{t("打开音频文件")}</button>
        <button class="btn" onclick={exportVideo} disabled={exporting || !task.audio_path}>
          {exporting ? t("导出中…") : t("导出视频")}
        </button>
        {#if task.video_path}
          <button class="btn" onclick={openVideo}>{t("打开视频文件")}</button>
        {/if}
        <button class="btn" onclick={resynthesize} disabled={resynthesizing}>
          {resynthesizing ? t("合成中…") : t("仅重新合成音频")}
        </button>
        <button class="btn btn-danger" onclick={remove}>{t("删除")}</button>
      </div>
    {:else if task.status === "failed"}
      <p class="err">{task.error}</p>
      <div class="actions">
        <button class="btn btn-danger" onclick={remove}>{t("删除")}</button>
      </div>
    {:else}
      <div class="actions">
        <button class="btn btn-danger" onclick={remove}>{t("删除")}</button>
      </div>
    {/if}
  {/if}

  {#if showTranscript && task.status === "completed"}
    {#if editing}
      <textarea class="transcript-edit" rows="18" bind:value={editBuf}></textarea>
      <p class="muted">
        {t("每行格式：")}<code>{t("PERSON_1: 台词")}</code> {t("或")} <code>{t("PERSON_2: 台词")}</code>{t("；没有前缀的行合成时会被跳过。")}
      </p>
      <div class="actions">
        <button class="btn btn-primary" onclick={saveEdit} disabled={saving}>
          {saving ? t("保存中…") : t("保存修改")}
        </button>
        <button class="btn" onclick={() => (editing = false)}>{t("取消编辑")}</button>
      </div>
    {:else}
      <div class="legend">
        <span class="chip p1">PERSON_1 · {t("{n} 条", { n: view.p1 })}</span>
        <span class="chip p2">PERSON_2 · {t("{n} 条", { n: view.p2 })}</span>
        <span class="chip plain">{t("共 {n} 条对白 / {m} 字", { n: view.p1 + view.p2, m: view.chars })}</span>
      </div>
      {#if view.ignored > 0}
        <p class="warn">
          {t("有 {n} 行缺少", { n: view.ignored })} <code>PERSON_1:</code> / <code>PERSON_2:</code>{t(" 前缀，合成语音时会被跳过（下面用虚线标出）。")}
        </p>
      {/if}
      {#if view.rows.length === 0}
        <p class="muted">{t("（暂无转录稿）")}</p>
      {:else}
        <div class="dialogue">
          {#each view.rows as row, i (i)}
            <div class="turn {row.speaker === null ? 'ignored' : row.speaker === 'PERSON_1' ? 'p1' : 'p2'}">
              <span class="who">
                {row.speaker === "PERSON_1" ? "1" : row.speaker === "PERSON_2" ? "2" : "?"}
              </span>
              <p class="say">{row.text}</p>
            </div>
          {/each}
        </div>
      {/if}
      <div class="actions">
        <button class="btn btn-primary" onclick={startEdit} disabled={!transcript}>
          {t("编辑转录稿")}
        </button>
      </div>
    {/if}
  {/if}
</article>

<style>
  .task {
    padding: var(--sp-4);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--sp-3);
    margin-bottom: var(--sp-2);
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }
  .title strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--fg-subtle);
    white-space: nowrap;
  }
  .progress {
    margin: var(--sp-2) 0 var(--sp-1);
  }
  .stage {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    margin: var(--sp-1) 0;
  }
  .eta {
    margin-left: var(--sp-1);
    color: var(--fg-subtle);
  }
  .audio {
    display: none;
  }
  .player {
    margin: var(--sp-2) 0;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
  }
  .ctl {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 30px;
    height: 28px;
    padding: 0 var(--sp-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    color: var(--fg);
    font: inherit;
    font-size: var(--fs-sm);
    line-height: 1;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s, box-shadow 0.15s;
  }
  .ctl:hover {
    background: var(--bg-inset);
    border-color: var(--fg-subtle);
  }
  .ctl:focus-visible {
    outline: 2px solid var(--brand);
    outline-offset: 2px;
  }
  .ctl.rate {
    min-width: 46px;
    font-variant-numeric: tabular-nums;
  }
  .ctl.rate.active {
    background: var(--brand);
    border-color: transparent;
    color: var(--brand-contrast);
  }
  .seek {
    flex: 1 1 160px;
    min-width: 120px;
    height: 28px;
    accent-color: var(--brand);
    cursor: pointer;
  }
  .seek:focus-visible {
    outline: 2px solid var(--brand);
    outline-offset: 2px;
    border-radius: var(--r-full);
  }
  .time {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
    flex-wrap: wrap;
  }
  .transcript-edit {
    width: 100%;
    margin: var(--sp-2) 0 0;
    padding: 10px;
    background: var(--bg-inset);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: var(--fs-sm);
    line-height: 1.6;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    margin: var(--sp-3) 0 var(--sp-2);
  }
  .chip {
    font-size: var(--fs-xs);
    padding: 3px 9px;
    border-radius: var(--r-full);
    border: 1px solid transparent;
    white-space: nowrap;
  }
  .chip.p1 {
    background: var(--spk1-bg);
    color: var(--spk1);
  }
  .chip.p2 {
    background: var(--spk2-bg);
    color: var(--spk2);
  }
  .chip.plain {
    background: var(--bg-inset);
    color: var(--fg-subtle);
  }
  .warn {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0 0 var(--sp-2);
    padding: 8px 12px;
    border-left: 3px solid var(--err);
    border-radius: var(--r-sm);
    background: var(--err-bg);
    color: var(--err-fg);
    font-size: var(--fs-sm);
  }
  .dialogue {
    max-height: 420px;
    overflow-y: auto;
    padding: var(--sp-1) 2px;
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    background: var(--bg-subtle);
  }
  .turn {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 7px 10px;
    border-left: 3px solid transparent;
    border-radius: var(--r-sm);
  }
  .turn + .turn {
    margin-top: 2px;
  }
  .turn.p1 {
    border-left-color: var(--spk1);
    background: var(--spk1-bg);
  }
  .turn.p2 {
    border-left-color: var(--spk2);
    background: var(--spk2-bg);
  }
  .turn.ignored {
    border-left: 3px dashed var(--err-border);
    background: var(--err-bg);
    color: var(--err-fg);
  }
  .who {
    flex: 0 0 18px;
    height: 18px;
    margin-top: 2px;
    border-radius: var(--r-full);
    background: var(--bg-elev);
    color: inherit;
    font-size: var(--fs-xs);
    line-height: 18px;
    text-align: center;
    font-weight: 700;
  }
  .say {
    margin: 0;
    font-size: var(--fs-md);
    line-height: 1.6;
    word-break: break-word;
  }
  /* 视频导出预览：与音频播放器同宽，黑底更接近成片观感 */
  .video {
    display: block;
    width: 100%;
    max-height: 420px;
    border-radius: var(--r-md);
    background: #000;
  }
</style>
