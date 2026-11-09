#!/usr/bin/env node
// Capture screenshots of the web UI through the Chrome DevTools Protocol,
// with no dependencies beyond Node (>= 22) and a Chromium-based browser.
//
//   node scripts/screenshots.mjs <base-url> <out-dir> name=/path [name=/path …]
//
// Environment: CHROME (browser binary), SCHEME (dark | light, default dark),
// WIDTH (1440), HEIGHT (900), SCALE (1), SETTLE_MS (2500).
import { spawn, spawnSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const [base, outDir, ...pages] = process.argv.slice(2)
if (!base || !outDir || pages.length === 0) {
  console.error('usage: screenshots.mjs <base-url> <out-dir> name=/path [name=/path …]')
  process.exit(2)
}
const env = process.env
const scheme = env.SCHEME || 'dark'
const width = Number(env.WIDTH || 1440)
const height = Number(env.HEIGHT || 900)
const scale = Number(env.SCALE || 1)
const settleMs = Number(env.SETTLE_MS || 2500)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

const chrome =
  env.CHROME ||
  ['chromium', 'chromium-browser', 'google-chrome-stable', 'google-chrome'].find(
    (bin) => spawnSync('which', [bin]).status === 0,
  )
if (!chrome) throw new Error('no Chromium/Chrome found; set CHROME=/path/to/browser')

const profile = await mkdtemp(join(tmpdir(), 'git-opt-shots-'))
const browser = spawn(
  chrome,
  [
    '--headless=new',
    '--disable-gpu',
    '--hide-scrollbars',
    '--no-first-run',
    '--no-default-browser-check',
    `--user-data-dir=${profile}`,
    '--remote-debugging-port=0',
    'about:blank',
  ],
  { stdio: 'ignore' },
)

try {
  let port
  for (let i = 0; i < 100 && !port; i++) {
    port = await readFile(join(profile, 'DevToolsActivePort'), 'utf8')
      .then((s) => s.split('\n')[0])
      .catch(() => sleep(100))
  }
  if (!port) throw new Error('browser did not start')

  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()
  const ws = new WebSocket(targets.find((t) => t.type === 'page').webSocketDebuggerUrl)
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve, { once: true })
    ws.addEventListener('error', reject, { once: true })
  })

  let nextId = 0
  const pending = new Map()
  const waiters = []
  ws.addEventListener('message', (event) => {
    const msg = JSON.parse(event.data)
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id)
      pending.delete(msg.id)
      msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result)
    } else if (msg.method) {
      for (const w of waiters.filter((w) => w.method === msg.method)) {
        waiters.splice(waiters.indexOf(w), 1)
        w.resolve(msg.params)
      }
    }
  })
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++nextId
      pending.set(id, { resolve, reject })
      ws.send(JSON.stringify({ id, method, params }))
    })
  const once = (method) => new Promise((resolve) => waiters.push({ method, resolve }))

  await send('Page.enable')
  await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: scale, mobile: false })
  await send('Emulation.setEmulatedMedia', {
    features: [
      { name: 'prefers-color-scheme', value: scheme },
      { name: 'prefers-reduced-motion', value: 'reduce' },
    ],
  })

  await mkdir(outDir, { recursive: true })
  for (const spec of pages) {
    const [name, path] = spec.split('=')
    const loaded = once('Page.loadEventFired')
    await send('Page.navigate', { url: new URL(path, base).href })
    await loaded
    await sleep(settleMs) // let the UI fetch its data and render
    const { data } = await send('Page.captureScreenshot', { format: 'png' })
    await writeFile(join(outDir, `${name}.png`), Buffer.from(data, 'base64'))
    console.log(`${name}.png  ${path}`)
  }
  ws.close()
} finally {
  const exited = new Promise((resolve) => browser.once('exit', resolve))
  browser.kill()
  await exited
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 })
}
