// Exports the app for the web for `npm run screenshots:sms -w @yuppers/mobile`:
// `support/export-web.mjs`, with the screenshot stack's harness proxy and
// server baked in and an export directory of its own. The ports are read
// from the same variables, with the same defaults, as playwright.config.ts.

import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

const result = spawnSync(process.execPath, [resolve(here, '../support/export-web.mjs')], {
  stdio: 'inherit',
  env: {
    ...process.env,
    E2E_MOBILE_PROXY_PORT: process.env.SCREENSHOTS_MOBILE_PROXY_PORT ?? '8336',
    E2E_MOBILE_WEB_PORT: process.env.SCREENSHOTS_MOBILE_WEB_PORT ?? '5335',
    E2E_MOBILE_EXPORT_DIR: resolve(here, '../.output/screenshots-web'),
  },
})
process.exit(result.status ?? 1)
