import { randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { expect, test, type Browser, type Locator, type Page } from '@playwright/test'
import type { Wording } from '@yuppers/shared'

import { SMS_CODE_CONSENT_VERSION } from '../../../../packages/shared/src/sms-code-consent.ts'
import {
  SAMPLE_PHONE,
  SCREENSHOT_WIDTH,
  screenshotPath,
  type Screen,
} from '../../../web/build/sms-opt-in.ts'
import { codeFrom } from '../support/codes'
import { repoRoot } from '../support/env'
import { phone } from '../support/fixtures'
import { en, es, fill } from '../support/wording'
import { apiURL, screenshotsLog, webURL } from './playwright.config'

/*
 * Takes the five pictures of the mobile app on the page on how people opt in
 * to texts, in each language, as a person on an iPhone goes through the
 * steps in the app:
 *
 *   1. the sign-in screen with a phone number entered, the box beside it not
 *      yet ticked and "Send code" waiting for it;
 *   2. the same with the box ticked and "Send code" enabled;
 *   3. the screen after asking for a code;
 *   4. "Text updates" on an agreement, the box not yet ticked;
 *   5. the same once ticked and saved, with the confirmation.
 *
 * Each is the whole screen, with the app's navigation bar, as the phone
 * shows it below its status bar. The screens are the app's own, run in the
 * browser harness; nothing of the browser is in the pictures.
 *
 * The person signs in with SAMPLE_PHONE, a number reserved for fiction,
 * whose code is read back from the API's log; someone else, through the
 * API, has sent them a proposal, which they open from its link. Their
 * account is deleted at the end, so the next run starts the same way. Each
 * picture is written as WebP, encoded by the browser itself.
 */

/** The screen the pictures are taken at: an iPhone's, at twice the density. */
const SCREEN = { width: SCREENSHOT_WIDTH, height: 844 }

/** The text message a code went out in, read back from the log, for a phone number. */
async function phoneCodeFrom(purpose: 'sign-in' | 'delete', send: () => Promise<unknown>) {
  const masked = `+1••••••••${SAMPLE_PHONE.slice(-2)}`
  const before = phoneCodes(masked, purpose).length
  await send()
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    const codes = phoneCodes(masked, purpose)
    if (codes.length > before) return codes[codes.length - 1]
    await new Promise((settle) => setTimeout(settle, 100))
  }
  throw new Error(`no text message with a code for ${masked} in ${screenshotsLog}`)
}

function phoneCodes(masked: string, purpose: 'sign-in' | 'delete'): string[] {
  const log = readFileSync(screenshotsLog, 'utf8')
  const lines = log.split('\n').filter((line) => line.includes('text message (development delivery)'))
  return lines
    .filter((line) => line.includes(masked))
    .filter((line) => (purpose === 'delete') === /delete|eliminar/.test(line))
    .map((line) => /text="?Yuppers\.app: (\d{6})/.exec(line)?.[1])
    .filter((code): code is string => code !== undefined)
}

/** A call to the API, straight to it, as someone outside the app. */
async function call(method: string, path: string, body?: unknown, token?: string) {
  const headers: Record<string, string> = { 'content-type': 'application/json' }
  if (token) headers.authorization = `Bearer ${token}`
  const response = await fetch(apiURL + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const text = await response.text()
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`)
  return text ? (JSON.parse(text) as unknown) : null
}

/** Someone with an email address who has sent a proposal bound to SAMPLE_PHONE. Returns its invitation token. */
async function proposal(): Promise<string> {
  const email = `m-opt-in-${randomUUID().slice(0, 8)}@example.test`
  const code = await codeFrom(
    email,
    'sign-in',
    () => call('POST', '/v1/auth/codes', { identifier: email }),
    screenshotsLog,
  )
  const { token } = (await call('POST', '/v1/auth/sessions', {
    identifier: email,
    code,
    delivery: 'TOKEN',
    language: 'en',
  })) as { token: string }
  await call('PATCH', '/v1/me', { display_name: 'Ana Ruiz', adult_confirmed: true }, token)
  const draft = (await call('POST', '/v1/exchanges', { timezone: 'America/Chicago' }, token)) as {
    id: string
    version: number
  }
  const sent = (await call(
    'POST',
    `/v1/exchanges/${draft.id}/revisions`,
    {
      expected_version: draft.version,
      consent: { language: 'en', version: 'draft-1' },
      invitation: { bound_to: SAMPLE_PHONE },
      terms: {
        party_a_name: 'Ana Ruiz',
        party_b_name: 'Sam Lee',
        terms: 'Repair the back fence.',
        contributions: [
          {
            id: randomUUID(),
            from: 'A',
            type: 'SERVICE',
            description: 'Repair the back fence',
            quantity: null,
            due: { kind: 'ON_AGREEMENT' },
            completion_criteria: null,
            required: true,
            amount_minor: null,
          },
        ],
      },
    },
    token,
  )) as { invitation_token: string }
  return sent.invitation_token
}

/** Writes a screenshot as WebP, encoded by the browser. */
async function save(browser: Browser, png: Buffer, screen: Screen, language: string) {
  const encoder = await browser.newPage()
  const base64 = await encoder.evaluate(async (data: string) => {
    const blob = await (await fetch(`data:image/png;base64,${data}`)).blob()
    const bitmap = await createImageBitmap(blob)
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
    canvas.getContext('2d')!.drawImage(bitmap, 0, 0)
    const webp = await canvas.convertToBlob({ type: 'image/webp', quality: 0.85 })
    const bytes = new Uint8Array(await webp.arrayBuffer())
    let text = ''
    for (const byte of bytes) text += String.fromCharCode(byte)
    return btoa(text)
  }, png.toString('base64'))
  await encoder.close()
  const path = screenshotPath(screen, language, 'en', 'mobile')
  writeFileSync(resolve(repoRoot, 'apps/web/public', path.slice(1)), Buffer.from(base64, 'base64'))
}

/** Only what is on screen: the browser keeps screens further down the stack in the page, hidden. */
const shown = (locator: Locator) => locator.filter({ visible: true })

/** Deletes the account signed in on `page`, with a code by text, freeing the number for the next run. */
async function deleteAccount(page: Page) {
  const token = await page.evaluate(() => window.sessionStorage.getItem('yuppers.harness.session'))
  if (!token) throw new Error('no session in the harness')
  const code = await phoneCodeFrom('delete', () =>
    call(
      'POST',
      '/v1/me/deletion/codes',
      {
        channel: 'PHONE',
        sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'en' },
      },
      token,
    ),
  )
  await call('POST', '/v1/me/deletion', { channel: 'PHONE', code }, token)
}

for (const [language, locale, w] of [
  ['en', 'en-US', en],
  ['es', 'es-ES', es],
] as [string, string, Wording][]) {
  test(`the opt-in screens of the mobile app in ${language}`, async ({ browser }) => {
    const token = await proposal()

    const context = await browser.newContext({
      ...phone,
      baseURL: webURL,
      locale,
      timezoneId: 'America/Chicago',
      viewport: SCREEN,
      screen: SCREEN,
      deviceScaleFactor: 2,
      isMobile: true,
      hasTouch: true,
      colorScheme: 'light',
    })
    const page = await context.newPage()

    // 1. The sign-in screen, offering phone numbers, with a number entered:
    // the consentBox beside it, unticked, and "Send code" waiting for it.
    await page.goto('/')
    await expect(shown(page.getByRole('heading', { name: w.signIn.title, level: 1 }))).toBeVisible()
    await expect(shown(page.getByText(fill(w.signIn.identifierHintCountries, { codes: '+1' })))).toBeVisible()
    const field = shown(page.getByLabel(w.signIn.identifierLabel))
    await field.fill(SAMPLE_PHONE)
    await field.blur()
    const consentBox = shown(page.getByRole('checkbox', { name: w.smsCode.signIn, exact: true }))
    const send = shown(page.getByRole('button', { name: w.signIn.sendCode, exact: true }))
    await expect(consentBox).not.toBeChecked()
    await expect(send).toBeDisabled()
    await expect(shown(page.getByText(w.smsCode.tickToSend, { exact: true }))).toBeVisible()
    // The form from its heading to the button, the consentBox in the middle.
    await consentBox.evaluate((element) => element.scrollIntoView({ block: 'center' }))
    await save(browser, await page.screenshot(), 'signIn', language)

    // 2. The consentBox ticked: "Send code" enabled.
    await consentBox.click()
    await expect(consentBox).toBeChecked()
    await expect(send).toBeEnabled()
    await consentBox.blur()
    await save(browser, await page.screenshot(), 'boxTicked', language)

    // 3. After asking for a code.
    const code = await phoneCodeFrom('sign-in', () => send.click())
    await expect(shown(page.getByText(fill(w.signIn.codeSent, { identifier: SAMPLE_PHONE })))).toBeVisible()
    const codeField = shown(page.getByLabel(w.signIn.codeLabel))
    await expect(codeField).toBeFocused()
    await codeField.blur()
    await save(browser, await page.screenshot(), 'codeSent', language)

    await codeField.fill(code)
    await shown(page.getByRole('button', { name: w.signIn.submit, exact: true })).click()
    await expect(shown(page.getByRole('heading', { name: w.profile.firstTitle }))).toBeVisible()
    await shown(page.getByLabel(w.profile.nameLabel, { exact: true })).fill('Sam Lee')
    await shown(page.getByLabel(w.profile.adultLabel, { exact: true })).check()
    await shown(page.getByRole('button', { name: w.profile.continue, exact: true })).click()
    await expect(shown(page.getByRole('heading', { name: w.home.title, level: 1 }))).toBeVisible()

    // The proposal sent to this number, opened from its link and taken up.
    await page.goto(`/${language}/i#${token}`)
    await expect(shown(page.getByRole('heading', { name: w.invitation.title, level: 1 }))).toBeVisible()
    await shown(
      page.getByRole('button', { name: fill(w.invitation.respondAs, { name: 'Sam Lee' }), exact: true }),
    ).click()
    await page.waitForURL(/\/exchanges\/[0-9a-f-]{36}$/)

    // 4. "Text updates", the box not yet ticked, at the top of the screen.
    const heading = shown(page.getByRole('heading', { name: w.smsUpdates.heading, level: 2 }))
    const box = shown(page.getByRole('checkbox', { name: w.smsUpdates.consent, exact: true }))
    await expect(box).not.toBeChecked()
    await heading.evaluate((element) => element.scrollIntoView({ block: 'start' }))
    // A little room above the heading, for the card's own edge.
    await heading.evaluate((element) => {
      let scroller = element.parentElement
      while (scroller && scroller.scrollHeight <= scroller.clientHeight) scroller = scroller.parentElement
      scroller?.scrollBy(0, -24)
    })
    await save(browser, await page.screenshot(), 'textUpdates', language)

    // 5. Ticked and saved.
    await box.click()
    await expect(box).toBeChecked()
    const saveButton = shown(page.getByRole('button', { name: w.smsUpdates.save, exact: true }))
    await saveButton.click()
    const on = fill(w.smsUpdates.on, { phone: `+1 •••-•••-${SAMPLE_PHONE.slice(-4)}` })
    await expect(shown(page.getByText(on, { exact: true }))).toBeVisible()
    await saveButton.blur()
    await save(browser, await page.screenshot(), 'confirmation', language)

    await deleteAccount(page)
    await context.close()
  })
}
