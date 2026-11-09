import { ref } from 'vue'
import { useToast } from './useToast'

/**
 * Wraps a button action: tracks which key is busy (to disable that button),
 * shows a toast on success or failure and runs `after` (usually a refresh).
 */
export function useAction(after?: () => Promise<void> | void) {
  const busy = ref<string | null>(null)
  const toast = useToast()

  async function run(key: string, fn: () => Promise<unknown>, success?: string) {
    if (busy.value) return
    busy.value = key
    try {
      await fn()
      if (success) toast.ok(success)
      await after?.()
    } catch (e) {
      toast.error(e)
    } finally {
      busy.value = null
    }
  }

  return { busy, run }
}
