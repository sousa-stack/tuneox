// Brief, non-blocking messages (command errors such as "only the master deck's
// tempo can change while synced").

export interface Toast {
  id: number;
  message: string;
}

type Listener = () => void;

const VISIBLE_MS = 3500;
let toasts: Toast[] = [];
let nextId = 1;
const listeners = new Set<Listener>();

function emit() {
  for (const listener of listeners) listener();
}

export function toast(error: unknown): void {
  const message = String(error instanceof Error ? error.message : error).replace(/^Error:\s*/, "");
  // Holding a key can repeat the same refusal; show it once.
  const existing = toasts.find((t) => t.message === message);
  if (existing) toasts = toasts.filter((t) => t !== existing);
  const item = { id: nextId++, message };
  toasts = [...toasts.slice(-3), item];
  emit();
  setTimeout(() => dismissToast(item.id), VISIBLE_MS);
}

export function dismissToast(id: number): void {
  const next = toasts.filter((t) => t.id !== id);
  if (next.length !== toasts.length) {
    toasts = next;
    emit();
  }
}

export const toastStore = {
  get: () => toasts,
  subscribe(listener: Listener) {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};
