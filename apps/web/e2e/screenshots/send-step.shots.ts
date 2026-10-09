import { mkdirSync } from 'node:fs'
import { resolve } from 'node:path'

import { expect, test, type Page } from '@playwright/test'

import { codeFrom } from '../support/codes'
import { webRoot } from '../support/env'
import { en } from '../support/wording'
import { screenshotsLog } from './playwright.config'

/*
 * Pictures of the step that sends the invitation link (DESIGN.md §8), for
 * review: the step for someone named by phone number, by email address and
 * for a link for anyone; the reminder on the exchange's page after "I’ll
 * send it later"; and the list with its chips. Each in light and dark, on a
 * phone-sized window. Not a test: it writes PNGs into SEND_STEP_SHOTS_DIR
 * (`e2e/.output/send-step-shots` unless set).
 *
 *   npx playwright test -c e2e/screenshots/playwright.config.ts send-step.shots.ts
 */

const OUT = resolve(process.env.SEND_STEP_SHOTS_DIR ?? resolve(webRoot, 'e2e/.output/send-step-shots'))
mkdirSync(OUT, { recursive: true })

const PHONE = { width: 390, height: 844 }

async function shoot(page: Page, name: string) {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme })
    await page.waitForTimeout(150)
    await page.screenshot({ path: resolve(OUT, `web-${name}-${scheme}.png`), fullPage: true })
  }
  await page.emulateMedia({ colorScheme: 'light' })
}

async function signUp(page: Page, name: string) {
  const email = `shots-${name.toLowerCase()}-${Date.now()}@example.test`
  await page.goto('/')
  await page.getByLabel(en.signIn.identifierLabel, { exact: true }).fill(email)
  const code = await codeFrom(
    email,
    'sign-in',
    () => page.getByRole('button', { name: en.signIn.sendCode, exact: true }).click(),
    screenshotsLog,
  )
  await page.getByLabel(en.signIn.codeLabel).fill(code)
  await page.getByRole('button', { name: en.signIn.submit, exact: true }).click()
  await page.getByLabel(en.profile.nameLabel, { exact: true }).fill(name)
  await page.getByLabel(en.profile.adultLabel).check()
  await page.getByRole('button', { name: en.profile.continue, exact: true }).click()
  await expect(page.getByRole('heading', { name: en.home.title, level: 1 })).toBeVisible()
}

/** Writes and signs a first proposal for Dana, named as `invitee`, landing on the step. */
async function propose(page: Page, invitee: string | null, description: string) {
  await page.goto('/')
  await page.getByRole('button', { name: en.home.start }).click()
  await expect(page.getByRole('heading', { name: en.composer.titleFirst, level: 1 })).toBeVisible()
  await page.getByLabel(en.composer.otherName).fill('Dana')
  if (invitee === null) {
    await page.getByRole('button', { name: en.invitationLink.forAnyone, exact: true }).click()
  } else {
    await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(invitee)
  }
  await page.getByRole('button', { name: en.composer.addYours }).click()
  await page.getByLabel(en.composer.descriptionLabel).fill(description)
  await page.getByRole('button', { name: en.composer.review, exact: true }).click()
  await page.getByLabel(en.consent.agree).check()
  await page.getByRole('button', { name: en.composer.signAndSend, exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Send it to Dana', level: 1 })).toBeVisible()
}

test('the step, the reminder and the list', async ({ browser }) => {
  const context = await browser.newContext({ viewport: PHONE, locale: 'en-US', timezoneId: 'UTC' })
  const page = await context.newPage()
  await signUp(page, 'Ana')

  // Named by phone: the step, then "later", the reminder and the list.
  await propose(page, '(202) 555-0142', 'A standing desk')
  await shoot(page, 'send-step-phone')
  await page.getByRole('button', { name: en.invitationLink.later, exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Yup with Dana', level: 1 })).toBeVisible()
  await shoot(page, 'reminder-later')
  await page.goto('/')
  await expect(page.locator('.cards .chip')).toHaveText(en.home.notSent)
  await shoot(page, 'home-not-sent')

  // Named by email: the step, and the way on once a way to send it is opened.
  await propose(page, 'dana@example.test', 'Two garden chairs')
  await shoot(page, 'send-step-email')
  await page.getByRole('button', { name: en.invitationLink.copy }).click()
  await expect(page.getByRole('button', { name: en.invitationLink.sendDone })).toBeVisible()
  await shoot(page, 'send-step-email-shared')
  await page.getByRole('button', { name: en.invitationLink.sendDone, exact: true }).click()
  await expect(page.getByText(/^You shared the link on /)).toBeVisible()
  await shoot(page, 'exchange-shared')
  await page.goto('/')
  await expect(page.locator('.cards .chip').first()).toBeVisible()
  await shoot(page, 'home-chips')

  // For anyone.
  await propose(page, null, 'A ride to the airport')
  await shoot(page, 'send-step-anyone')
  await context.close()
})
