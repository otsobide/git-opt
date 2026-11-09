// Presentation helpers (es-ES).

const number = new Intl.NumberFormat('es-ES')
const decimal = new Intl.NumberFormat('es-ES', { maximumFractionDigits: 1 })
const relative = new Intl.RelativeTimeFormat('es-ES', { numeric: 'auto' })
const dateTime = new Intl.DateTimeFormat('es-ES', { dateStyle: 'medium', timeStyle: 'short' })

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB']

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return '—'
  const sign = bytes < 0 ? '−' : ''
  let value = Math.abs(bytes)
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit++
  }
  const text = unit === 0 ? number.format(value) : decimal.format(value)
  return `${sign}${text} ${UNITS[unit]}`
}

export function formatNumber(n: number): string {
  return number.format(n)
}

export function savedPercent(before: number, after: number): number {
  if (!before) return 0
  return ((before - after) / before) * 100
}

export function formatPercent(p: number): string {
  return `${decimal.format(p)} %`
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—'
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? '—' : dateTime.format(d)
}

/** "hace 5 min", "en 12 min"… relative to `now` (ms). */
export function formatRelative(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return '—'
  const t = new Date(iso).getTime()
  if (Number.isNaN(t)) return '—'
  const diff = (t - now) / 1000
  const abs = Math.abs(diff)
  if (abs < 45) return diff >= 0 ? 'en unos segundos' : 'hace un momento'
  if (abs < 3600) return relative.format(Math.round(diff / 60), 'minute')
  if (abs < 86400) return relative.format(Math.round(diff / 3600), 'hour')
  return relative.format(Math.round(diff / 86400), 'day')
}

/** Duration between two timestamps, e.g. "1 min 12 s". */
export function formatDuration(start: string, end: string | null): string {
  if (!end) return '—'
  const s = Math.max(0, Math.round((new Date(end).getTime() - new Date(start).getTime()) / 1000))
  if (s < 60) return `${s} s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} min ${s % 60} s`
  return `${Math.floor(m / 60)} h ${m % 60} min`
}

export function shortSha(sha: string | null | undefined): string {
  return sha ? sha.slice(0, 7) : '—'
}

export function minutes(m: number): string {
  if (m < 60) return `${m} min`
  const h = Math.floor(m / 60)
  const rest = m % 60
  return rest ? `${h} h ${rest} min` : `${h} h`
}
