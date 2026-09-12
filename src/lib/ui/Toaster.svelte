<script lang="ts">
  import { toasts, dismiss } from "./feedback.svelte.js";
  import { t } from "../i18n.svelte.js";
</script>

<div class="toaster" aria-live="polite">
  {#each toasts as toast (toast.id)}
    <div class="toast {toast.kind}" role={toast.kind === "err" ? "alert" : "status"}>
      <span class="text">{toast.text}</span>
      <button class="close" onclick={() => dismiss(toast.id)} aria-label={t("关闭通知")}>×</button>
    </div>
  {/each}
</div>

<style>
  .toaster {
    position: fixed;
    right: var(--sp-4);
    bottom: var(--sp-4);
    z-index: 100;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    max-width: 380px;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--fg-subtle);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    color: var(--fg);
    box-shadow: var(--shadow-3);
    font-size: var(--fs-md);
    animation: toast-in 0.18s ease-out;
  }
  .toast.ok {
    border-left-color: var(--ok-fg);
  }
  .toast.err {
    border-left-color: var(--err);
  }
  .text {
    flex: 1;
    word-break: break-word;
  }
  .close {
    border: none;
    background: none;
    color: var(--fg-subtle);
    font-size: 1rem;
    line-height: 1;
    padding: 0 2px;
    cursor: pointer;
  }
  .close:hover {
    color: var(--fg);
  }
  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .toast {
      animation: none;
    }
  }
</style>
