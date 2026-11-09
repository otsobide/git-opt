import { readonly, ref } from 'vue'

// One shared clock for relative timestamps ("in 7 minutes").
const now = ref(Date.now())
let started = false

export function useNow() {
  if (!started) {
    started = true
    setInterval(() => {
      now.value = Date.now()
    }, 15_000)
  }
  return readonly(now)
}
