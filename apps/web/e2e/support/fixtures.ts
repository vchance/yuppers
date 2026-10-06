import { randomUUID } from 'node:crypto'

import { expect, test as base, type BrowserContext, type Page } from '@playwright/test'

/*
 * The people in a test. Each has a browser context of their own, so two
 * people never share a cookie, and an `example.test` address nobody else
 * uses, so tests can run side by side against one database.
 *
 * Every page the service serves carries a strict Content-Security-Policy
 * (backend/src/http/mod.rs). Anything a page tries that the policy refuses,
 * such as an inline style or script, is collected from each person's
 * browser and fails the test.
 *
 * Every browser here is a desktop one without a share sheet of its own, as
 * most desktop browsers are: `navigator.share` is taken away, whatever the
 * machine running the tests offers, so "Share link" opens the app's own
 * panel of ways to share in every run alike.
 */

export interface Person {
  /** The name they give their account. */
  name: string
  email: string
  context: BrowserContext
  page: Page
}

interface PersonOptions {
  /** The browser's language. English unless given. */
  locale?: string
  /** The window's size, in CSS pixels. Playwright's desktop default unless given. */
  viewport?: { width: number; height: number }
  /** The device's time zone. UTC unless given. */
  timezoneId?: string
  /** Whether pages may run scripts. They may unless this says otherwise. */
  javaScriptEnabled?: boolean
}

interface Fixtures {
  /** A new person with a browser of their own, signed out. */
  person(name: string, options?: PersonOptions): Promise<Person>
}

export const test = base.extend<Fixtures>({
  // The second argument hands the fixture to the test; it is Playwright's
  // `use`, renamed so it is not taken for a React hook.
  person: async ({ browser, baseURL }, provide, testInfo) => {
    const contexts: BrowserContext[] = []
    const refused: string[] = []
    // Unique per test, and readable in the log.
    const run = randomUUID().slice(0, 8)
    await provide(async (name, options = {}) => {
      const context = await browser.newContext({
        baseURL,
        locale: options.locale ?? 'en-US',
        ...(options.viewport ? { viewport: options.viewport } : {}),
        timezoneId: options.timezoneId ?? 'UTC',
        javaScriptEnabled: options.javaScriptEnabled ?? true,
        permissions: ['clipboard-read', 'clipboard-write'],
      })
      contexts.push(context)
      await context.exposeBinding('reportRefusedByPolicy', (_source, what: string) => {
        refused.push(`${name}: ${what}`)
      })
      await context.addInitScript(() => {
        Reflect.deleteProperty(Navigator.prototype, 'share')
        Reflect.deleteProperty(Navigator.prototype, 'canShare')
        document.addEventListener('securitypolicyviolation', (event) => {
          const report = (window as unknown as { reportRefusedByPolicy(what: string): void })
            .reportRefusedByPolicy
          report(`${event.violatedDirective} ${event.blockedURI} at ${event.sourceFile}:${event.lineNumber}`)
        })
      })
      const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-')
      const email = `${slug}-w${testInfo.workerIndex}-${run}@example.test`
      return { name, email, context, page: await context.newPage() }
    })
    for (const context of contexts) await context.close()
    expect(refused, 'refused by the Content-Security-Policy').toEqual([])
  },
})

export { expect }
