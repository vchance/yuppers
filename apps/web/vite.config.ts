import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import react from '@vitejs/plugin-react'
import type { Plugin } from 'vite'
import { defineConfig } from 'vitest/config'

import { entryPagesPlugin } from './build/entry-pages.ts'
import { MANIFEST_IN_BUILD, manifestOutPlugin } from './build/manifest.ts'

// The build names itself to the service on every request, so a build too old
// to act can be told so (`CLIENT_TOO_OLD`). The version is the package's.
const { version } = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
) as { version: string }

// The commit the build is made from, when its environment says: GIT_SHA from
// the Dockerfile's build argument or CI, or RENDER_GIT_COMMIT on Render. Shown
// on the account and staff screens and sent with the version. Never read
// from git itself, so a build from a plain copy of the source still works.
const commitFromEnvironment = [process.env.GIT_SHA, process.env.RENDER_GIT_COMMIT]
  .map((value) => value?.trim() ?? '')
  .find((value) => /^[0-9a-fA-F]{7,64}$/.test(value))
const commit = commitFromEnvironment ? commitFromEnvironment.toLowerCase() : null

// The Rust service in development. In production the web app is static files
// served from the same origin as the API. Set API_PROXY_TARGET when the
// service listens somewhere other than its default address; whatever origin
// this dev server runs on must also be the service's WEB_ORIGIN, or it will
// not honor the session cookie.
const api = process.env.API_PROXY_TARGET ?? 'http://127.0.0.1:8080'

const wording = fileURLToPath(new URL('../../packages/shared/wording', import.meta.url))

// Read by `scripts/check-budget.mjs`; kept out of `dist`, which is served.
const manifest = fileURLToPath(new URL('./.build/manifest.json', import.meta.url))

/**
 * Leaves out of the web app's copy of each language's wording what only the
 * service sends: the emails (`notifications`) and the text messages (`sms`),
 * which no screen shows. The service reads the files whole
 * (`backend/src/notifications/wording.rs`), and so does the mobile app; this
 * keeps them off the invitation page's first load (`scripts/check-budget.mjs`).
 */
function serviceWordingOut(): Plugin {
  const SERVICE_ONLY = ['notifications', 'sms']
  return {
    name: 'yuppers:service-wording-out',
    enforce: 'pre',
    transform(code, id) {
      if (!/packages\/shared\/wording\/[a-z]{2,3}(-[A-Za-z0-9]{2,8})*\.json$/.test(id)) return null
      const wording = JSON.parse(code) as Record<string, unknown>
      for (const key of SERVICE_ONLY) delete wording[key]
      return { code: JSON.stringify(wording), map: null }
    },
  }
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    serviceWordingOut(),
    react(),
    entryPagesPlugin(wording),
    manifestOutPlugin(manifest),
  ],
  build: { manifest: MANIFEST_IN_BUILD },
  define: { __WEB_VERSION__: JSON.stringify(version), __WEB_COMMIT__: JSON.stringify(commit) },
  server: {
    proxy: {
      '/v1': api,
      '/healthz': api,
      '/readyz': api,
    },
  },
  test: {
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx', 'build/**/*.test.ts'],
  },
})
