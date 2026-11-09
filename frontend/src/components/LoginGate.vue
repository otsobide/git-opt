<script setup lang="ts">
import { ref } from 'vue'
import { api, authRequired, setAdminToken } from '@/api'
import BrandMark from './BrandMark.vue'

const emit = defineEmits<{ ok: [] }>()

const token = ref('')
const error = ref<string | null>(null)
const busy = ref(false)

async function submit() {
  if (!token.value.trim()) return
  busy.value = true
  error.value = null
  setAdminToken(token.value.trim())
  authRequired.value = false
  try {
    await api.status()
    emit('ok')
  } catch {
    setAdminToken(null)
    authRequired.value = true
    error.value = 'Invalid token. Check GIT_OPT_ADMIN_TOKEN on the server.'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="gate">
    <form class="card" @submit.prevent="submit">
      <BrandMark />
      <h1>Admin access</h1>
      <p class="muted small">
        This server requires the token set in <code>GIT_OPT_ADMIN_TOKEN</code>.
      </p>
      <div class="field">
        <label for="admin-token">Token</label>
        <input
          id="admin-token"
          v-model="token"
          class="input"
          type="password"
          autocomplete="current-password"
          autofocus
        />
      </div>
      <p v-if="error" class="error-text">{{ error }}</p>
      <button class="btn btn-primary" type="submit" :disabled="busy || !token.trim()">Sign in</button>
    </form>
  </div>
</template>

<style scoped>
.gate {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 1rem;
}
.card {
  display: grid;
  gap: 1rem;
  width: min(24rem, 100%);
  padding: 1.75rem;
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
}
h1 {
  font-size: var(--fs-xl);
}
</style>
