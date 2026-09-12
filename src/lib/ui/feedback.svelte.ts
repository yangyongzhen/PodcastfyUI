// 应用内反馈原语：Toast 通知 + Promise 化的确认弹窗。
// 用于替代 alert()/confirm()——Tauri 窗口里原生弹窗样式不可控，且在部分平台行为不一致。

import { t } from "../i18n.svelte.js";

export type ToastKind = "info" | "ok" | "err";

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

let seq = 0;

export const toasts = $state<Toast[]>([]);

/** 弹一条通知；ttl=0 表示不自动消失。 */
export function toast(text: string, kind: ToastKind = "info", ttl = 4500): number {
  const id = ++seq;
  toasts.push({ id, kind, text });
  if (ttl > 0) setTimeout(() => dismiss(id), ttl);
  return id;
}

export function dismiss(id: number): void {
  const i = toasts.findIndex((t) => t.id === id);
  if (i !== -1) toasts.splice(i, 1);
}

interface ConfirmState {
  open: boolean;
  title: string;
  message: string;
  confirmText: string;
  danger: boolean;
  resolve: ((value: boolean) => void) | null;
}

export const confirmState = $state<ConfirmState>({
  open: false,
  title: t("确认"),
  message: "",
  confirmText: t("确定"),
  danger: false,
  resolve: null,
});

/** 等待用户确认；返回 true 表示确认，false/关闭表示取消。 */
export function confirmDialog(
  message: string,
  opts: { title?: string; confirmText?: string; danger?: boolean } = {},
): Promise<boolean> {
  // 上一个弹窗还挂着 Promise 时先当取消处理，避免 Promise 永久悬挂。
  confirmState.resolve?.(false);
  return new Promise<boolean>((resolve) => {
    confirmState.title = opts.title ?? t("确认");
    confirmState.message = message;
    confirmState.confirmText = opts.confirmText ?? t("确定");
    confirmState.danger = opts.danger ?? false;
    confirmState.resolve = resolve;
    confirmState.open = true;
  });
}

export function resolveConfirm(value: boolean): void {
  const resolve = confirmState.resolve;
  confirmState.open = false;
  confirmState.resolve = null;
  resolve?.(value);
}
