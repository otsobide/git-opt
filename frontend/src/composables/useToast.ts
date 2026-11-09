import { ref } from 'vue'

export interface Toast {
  id: number
  kind: 'ok' | 'error'
  text: string
}

const toasts = ref<Toast[]>([])
let next = 1

function push(kind: Toast['kind'], text: string, ms: number) {
  const id = next++
  toasts.value.push({ id, kind, text })
  setTimeout(() => dismiss(id), ms)
}

function dismiss(id: number) {
  toasts.value = toasts.value.filter((t) => t.id !== id)
}

export function useToast() {
  return {
    toasts,
    dismiss,
    ok: (text: string) => push('ok', text, 3500),
    error: (e: unknown) => push('error', e instanceof Error ? e.message : String(e), 7000),
  }
}
