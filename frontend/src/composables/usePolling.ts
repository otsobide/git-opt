import { onBeforeUnmount, onMounted, ref } from 'vue'

/**
 * Calls `load` on mount and then every `intervalMs` while the tab is visible.
 * Errors are kept in `error` instead of thrown, so a flaky request does not
 * blank the page.
 */
export function usePolling(load: () => Promise<void>, intervalMs = 5000) {
  const loading = ref(true)
  const error = ref<string | null>(null)
  let timer: ReturnType<typeof setTimeout> | null = null
  let stopped = false
  let inFlight = false

  async function refresh() {
    if (inFlight) return
    inFlight = true
    try {
      await load()
      error.value = null
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
    } finally {
      loading.value = false
      inFlight = false
    }
  }

  function schedule() {
    if (stopped) return
    timer = setTimeout(async () => {
      if (document.visibilityState === 'visible') await refresh()
      schedule()
    }, intervalMs)
  }

  function onVisibility() {
    if (document.visibilityState === 'visible') void refresh()
  }

  onMounted(() => {
    void refresh()
    schedule()
    document.addEventListener('visibilitychange', onVisibility)
  })

  onBeforeUnmount(() => {
    stopped = true
    if (timer) clearTimeout(timer)
    document.removeEventListener('visibilitychange', onVisibility)
  })

  return { loading, error, refresh }
}
