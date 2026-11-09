import { ref, watch } from 'vue'

export type Theme = 'system' | 'light' | 'dark'

// Same key as the inline script in index.html, which applies the theme
// before the first paint to avoid a flash of the wrong colors.
const KEY = 'git-opt.theme'

function load(): Theme {
  try {
    const value = localStorage.getItem(KEY)
    return value === 'light' || value === 'dark' ? value : 'system'
  } catch {
    return 'system'
  }
}

const theme = ref<Theme>(load())

function apply(value: Theme) {
  const root = document.documentElement
  if (value === 'system') delete root.dataset.theme
  else root.dataset.theme = value
  try {
    if (value === 'system') localStorage.removeItem(KEY)
    else localStorage.setItem(KEY, value)
  } catch {
    // Storage may be unavailable (private mode); the choice lasts for this tab.
  }
}

watch(theme, apply, { immediate: true })

/** Color theme: follows the OS unless the user forces light or dark. */
export function useTheme() {
  return { theme }
}
