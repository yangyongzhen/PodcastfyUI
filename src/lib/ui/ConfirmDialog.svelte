<script lang="ts">
  import { confirmState, resolveConfirm } from "./feedback.svelte.js";
  import { t } from "../i18n.svelte.js";

  // 只处理 Esc（取消）。不绑定 Enter：破坏性操作下误触回车太危险。
  function onKeydown(e: KeyboardEvent) {
    if (confirmState.open && e.key === "Escape") resolveConfirm(false);
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if confirmState.open}
  <div class="overlay">
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-label={confirmState.title}
    >
      <h3>{confirmState.title}</h3>
      <p>{confirmState.message}</p>
      <div class="actions">
        <button class="btn" onclick={() => resolveConfirm(false)}>{t("取消")}</button>
        <button
          class={confirmState.danger ? "btn btn-danger" : "btn btn-primary"}
          onclick={() => resolveConfirm(true)}
        >
          {confirmState.confirmText}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--sp-5);
    background: rgba(10, 12, 20, 0.45);
  }
  .dialog {
    width: min(420px, 100%);
    padding: var(--sp-5);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--bg-elev);
    box-shadow: var(--shadow-3);
  }
  .dialog h3 {
    margin: 0 0 var(--sp-2);
    font-size: var(--fs-lg);
  }
  .dialog p {
    margin: 0 0 var(--sp-4);
    color: var(--fg-muted);
    font-size: var(--fs-md);
    word-break: break-word;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-2);
  }
</style>
