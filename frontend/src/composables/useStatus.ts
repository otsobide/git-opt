import { ref } from 'vue'
import { api } from '@/api'
import type { Status } from '@/types'

// Shared server status (identity, optimizers, jobs). App.vue polls it; any
// view can read it or ask for an immediate refresh after an action.
const status = ref<Status | null>(null)
const statusError = ref<string | null>(null)

async function refreshStatus() {
  try {
    status.value = await api.status()
    statusError.value = null
  } catch (e) {
    statusError.value = e instanceof Error ? e.message : String(e)
  }
}

export function useStatus() {
  return { status, statusError, refreshStatus }
}
