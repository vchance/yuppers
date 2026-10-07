import { resolve } from 'node:path'

import { defineConfig } from '@playwright/test'

import { apiBinary, apiEnvironment, mobileRoot, repoRoot } from '../support/env'

/*
 * `npm run screenshots:sms -w @yuppers/mobile` (or `npm run
 * screenshots:sms:mobile` from the root): the pictures of the mobile app on
 * the page on how people opt in to texts (`apps/web/build/sms-opt-in.ts`,
 * "In the Yuppers mobile app"). Not a test: it writes the pictures into
 * `apps/web/public/sms-opt-in/mobile/`.
 *
 * The app's own screens, exported for the web by `export.mjs` and run in the
 * browser harness (README, "Running the screens in a browser"), in Chromium
 * the size of an iPhone. It runs against a stack of its own: the API on
 * 8335, with text messages written to its log (`SMS_DELIVERY=log`, `SMS_CODE_DELIVERY=log`) and phone
 * sign-in on, the harness proxy on 8336 and the export's server on 5335.
 * The database is the one in `.env`, with the migrations applied. Nothing is
 * sent anywhere.
 *
 * The ports are read from the same variables, with the same defaults, as
 * `export.mjs`, which bakes the proxy's address into the export.
 */

const host = '127.0.0.1'
export const apiPort = Number(process.env.SCREENSHOTS_MOBILE_API_PORT ?? 8335)
const proxyPort = Number(process.env.SCREENSHOTS_MOBILE_PROXY_PORT ?? 8336)
const webPort = Number(process.env.SCREENSHOTS_MOBILE_WEB_PORT ?? 5335)
export const apiURL = `http://${host}:${apiPort}`
export const webURL = `http://${host}:${webPort}`
const proxyURL = `http://${host}:${proxyPort}`

export const exportDir = resolve(mobileRoot, 'e2e/.output/screenshots-web')
export const screenshotsLog = resolve(mobileRoot, 'e2e/.output/screenshots-api.log')

export default defineConfig({
  testDir: '.',
  testMatch: '*.shots.ts',
  outputDir: '../../test-results/screenshots',
  fullyParallel: false,
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  use: { baseURL: webURL, trace: 'retain-on-failure' },
  webServer: [
    {
      command: `exec "${apiBinary}" > "${screenshotsLog}" 2>&1`,
      cwd: repoRoot,
      url: `${apiURL}/readyz`,
      env: {
        ...apiEnvironment(),
        BIND_ADDR: `${host}:${apiPort}`,
        WEB_ORIGIN: webURL,
        // Codes for phone numbers in the log, where Twilio Verify would text
        // them, and agreement updates offered: what the screens show when
        // texting is on.
        SMS_DELIVERY: 'log',
        SMS_CODE_DELIVERY: 'log',
        SMS_MAX_PER_PREFIX_PER_HOUR: '1000',
      },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: 'node scripts/harness-proxy.mjs',
      cwd: mobileRoot,
      url: `${proxyURL}/healthz`,
      env: { API_TARGET: apiURL, HARNESS_ORIGIN: webURL, PORT: String(proxyPort) },
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command: 'node e2e/support/serve-export.mjs',
      cwd: mobileRoot,
      url: webURL,
      env: { EXPORT_DIR: exportDir, PORT: String(webPort) },
      reuseExistingServer: false,
      timeout: 30_000,
    },
  ],
})
