import { mkdirSync } from 'node:fs'
import { resolve } from 'node:path'

import { devices, expect, test, type Page } from '@playwright/test'

import { codeFrom } from '../support/codes'
import { mobileRoot } from '../support/env'
import { en } from '../support/wording'
import { screenshotsLog } from './playwright.config'

/*
 * Pictures of the mobile app's step that sends the invitation link
 * (DESIGN.md §8), for review, through the browser harness: the step for
 * someone named by phone number, by email address and for a link for
 * anyone; the reminder on the exchange's screen after "I’ll send it later";
 * and the list with its chips. Each in light and dark. Not a test: it
 * writes PNGs into SEND_STEP_SHOTS_DIR (`e2e/.output/send-step-shots`
 * unless set).
 *
 *   node e2e/screenshots/export.mjs
 *   npx playwright test -c e2e/screenshots/playwright.config.ts send-step.shots.ts
 */

const OUT = resolve(
  process.env.SEND_STEP_SHOTS_DIR ?? resolve(mobileRoot, 'e2e/.output/send-step-shots'),
)
mkdirSync(OUT, { recursive: true })

const { defaultBrowserType: _browser, ...phone } = devices['iPhone 15']

const shown = (page: Page, selector: Parameters<Page['getByRole']>[0], name: string | RegExp) =>
  page.getByRole(selector, { name, exact: typeof name === 'string' }).filter({ visible: true })

async function shoot(page: Page, name: string) {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme })
    await page.waitForTimeout(250)
    await page.screenshot({ path: resolve(OUT, `mobile-${name}-${scheme}.png`), fullPage: true })
  }
  await page.emulateMedia({ colorScheme: 'light' })
}

async function signUp(page: Page, name: string) {
  const email = `m-shots-${name.toLowerCase()}-${Date.now()}@example.test`
  await page.goto('/')
  await page.getByLabel(en.signIn.identifierLabel).filter({ visible: true }).fill(email)
  const code = await codeFrom(
    email,
    'sign-in',
    () => shown(page, 'button', en.signIn.sendCode).click(),
    screenshotsLog,
  )
  await page.getByLabel(en.signIn.codeLabel).filter({ visible: true }).fill(code)
  await shown(page, 'button', en.signIn.submit).click()
  await page.getByLabel(en.profile.nameLabel, { exact: true }).filter({ visible: true }).fill(name)
  await page.getByLabel(en.profile.adultLabel, { exact: true }).filter({ visible: true }).check()
  await shown(page, 'button', en.profile.continue).click()
  await expect(page.getByRole('heading', { name: en.home.title, level: 1 }).last()).toBeVisible()
}

/** Writes and signs a first proposal for Dana, named as `invitee`, landing on the step. */
async function propose(page: Page, invitee: string | null, description: string) {
  await page.goto('/')
  await shown(page, 'button', en.home.start).click()
  await expect(
    page.getByRole('heading', { name: en.composer.titleFirst, level: 1 }).last(),
  ).toBeVisible()
  await page.getByLabel(en.composer.otherName, { exact: true }).fill('Dana')
  if (invitee === null) {
    await shown(page, 'link', en.invitationLink.forAnyone).click()
  } else {
    await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(invitee)
  }
  await shown(page, 'button', en.composer.addYours).click()
  await page.getByLabel(en.composer.descriptionLabel, { exact: true }).fill(description)
  await shown(page, 'button', en.composer.review).click()
  await expect(
    page.getByRole('heading', { name: en.composer.signTitle, level: 1 }).last(),
  ).toBeVisible()
  await page.getByLabel(en.consent.agree, { exact: true }).check()
  await page.getByTestId('consent-sign').click()
  await expect(page.getByRole('heading', { name: 'Send it to Dana', level: 1 }).last()).toBeVisible()
}

test('the step, the reminder and the list', async ({ browser }) => {
  const context = await browser.newContext({ ...phone, locale: 'en-US', timezoneId: 'UTC' })
  const page = await context.newPage()
  await signUp(page, 'Ana')

  // Named by phone: the step, then "later", the reminder and the list.
  await propose(page, '(202) 555-0142', 'A standing desk')
  await shoot(page, 'send-step-phone')
  await shown(page, 'link', en.invitationLink.later).click()
  await expect(
    page.getByRole('heading', { name: 'Yup with Dana', level: 1 }).last(),
  ).toBeVisible()
  await shoot(page, 'reminder-later')
  await page.goto('/')
  await expect(page.getByText(en.home.notSent)).toBeVisible()
  await shoot(page, 'home-not-sent')

  // Named by email: the step, and the way on once a way to send it is opened.
  await propose(page, 'dana@example.test', 'Two garden chairs')
  await shoot(page, 'send-step-email')
  await shown(page, 'button', en.invitationLink.copy).click()
  await expect(shown(page, 'button', en.invitationLink.sendDone)).toBeVisible()
  await shoot(page, 'send-step-email-shared')
  await shown(page, 'button', en.invitationLink.sendDone).click()
  await expect(page.getByText(/^You shared the link on /)).toBeVisible()
  await shoot(page, 'exchange-shared')
  await page.goto('/')
  await expect(page.getByText(/^Waiting for Dana$/)).toBeVisible()
  await shoot(page, 'home-chips')

  // For anyone.
  await propose(page, null, 'A ride to the airport')
  await shoot(page, 'send-step-anyone')
  await context.close()
})
