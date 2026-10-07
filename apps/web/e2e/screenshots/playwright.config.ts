import { mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

import { defineConfig } from '@playwright/test'

import { apiBinary, repoRoot, signInLimits, webRoot } from '../support/env'

/*
 * `npm run screenshots:sms`: the pictures on the page on how people opt in
 * to texts (`build/sms-opt-in.ts`), taken from the web app as a person on a
 * phone sees it. Not a test: it writes the pictures into
 * `public/sms-opt-in/` (README, "Privacy policy and terms").
 *
 * It runs against a stack of its own: the API on 8331, with text messages
 * written to its log (`SMS_DELIVERY=log`, `SMS_CODE_DELIVERY=log`) and phone sign-in on, and the web
 * app's development server on 5331 in front of it. The database is the one
 * in `.env`, with the migrations applied. Nothing is sent anywhere.
 */

export const apiPort = Number(process.env.SCREENSHOTS_API_PORT ?? 8331)
export const webPort = Number(process.env.SCREENSHOTS_WEB_PORT ?? 5331)
/** `localhost`, as the web app's development server expects, and the API's WEB_ORIGIN with it. */
export const webOrigin = `http://localhost:${webPort}`
export const screenshotsLog = resolve(webRoot, 'e2e/.output/screenshots-api.log')
mkdirSync(dirname(screenshotsLog), { recursive: true })

export default defineConfig({
  testDir: '.',
  testMatch: '*.shots.ts',
  outputDir: '../../test-results/screenshots',
  fullyParallel: false,
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  use: { baseURL: webOrigin, trace: 'retain-on-failure' },
  webServer: [
    {
      command: `exec "${apiBinary}" > "${screenshotsLog}" 2>&1`,
      cwd: repoRoot,
      url: `http://127.0.0.1:${apiPort}/readyz`,
      env: {
        BIND_ADDR: `127.0.0.1:${apiPort}`,
        WEB_ORIGIN: webOrigin,
        CODE_DELIVERY: 'log',
        NOTIFICATION_DELIVERY: 'log',
        // Codes for phone numbers in the log, where Twilio Verify would text
        // them, and agreement updates offered: what the screens show when
        // texting is on.
        SMS_DELIVERY: 'log',
        SMS_CODE_DELIVERY: 'log',
        SMS_MAX_PER_PREFIX_PER_HOUR: '1000',
        RUST_LOG: 'info',
        NO_COLOR: '1',
        ...signInLimits,
      },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: `npx vite --port ${webPort} --strictPort`,
      cwd: webRoot,
      url: webOrigin,
      env: { API_PROXY_TARGET: `http://127.0.0.1:${apiPort}` },
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
})
