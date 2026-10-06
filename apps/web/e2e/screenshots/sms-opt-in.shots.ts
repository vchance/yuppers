import { randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { expect, test, type APIRequestContext, type Browser, type Page } from '@playwright/test'
import type { Wording } from '@yuppers/shared'

import {
  SAMPLE_PHONE,
  SCREENSHOT_WIDTH,
  screenshotPath,
  type Screen,
} from '../../build/sms-opt-in'
import { codeFrom } from '../support/codes'
import { webRoot } from '../support/env'
import { en, es, fill } from '../support/wording'
import { apiPort, screenshotsLog, webOrigin } from './playwright.config'

/*
 * Takes the four pictures of the page on how people opt in to texts, in each
 * language, as a person on a phone goes through the steps:
 *
 *   1. the sign-in form with a phone number entered;
 *   2. the form after asking for a code;
 *   3. "Text updates" on an agreement, the box not yet ticked;
 *   4. the same once ticked and saved.
 *
 * The person signs in with SAMPLE_PHONE, a number reserved for fiction,
 * whose code is read back from the API's log; someone else, through the
 * API, has sent them a proposal. Their account is deleted at the end, so
 * the next run starts the same way. Each picture is written as WebP,
 * encoded by the browser itself.
 */

const API = `http://127.0.0.1:${apiPort}`

/** The text message a code went out in, read back from the log, for a phone number. */
function phoneCodeFrom(purpose: 'sign-in' | 'delete', send: () => Promise<unknown>) {
  const masked = `+1••••••••${SAMPLE_PHONE.slice(-2)}`
  const before = phoneCodes(masked, purpose).length
  return (async () => {
    await send()
    const deadline = Date.now() + 15_000
    while (Date.now() < deadline) {
      const codes = phoneCodes(masked, purpose)
      if (codes.length > before) return codes[codes.length - 1]
      await new Promise((settle) => setTimeout(settle, 100))
    }
    throw new Error(`no text message with a code for ${masked} in ${screenshotsLog}`)
  })()
}

function phoneCodes(masked: string, purpose: 'sign-in' | 'delete'): string[] {
  const log = readFileSync(screenshotsLog, 'utf8')
  const lines = log.split('\n').filter((line) => line.includes('text message (development delivery)'))
  return lines
    .filter((line) => line.includes(masked))
    .filter((line) => (purpose === 'delete') === /delete|eliminar/.test(line))
    .map((line) => /text="?(\d{6})/.exec(line)?.[1])
    .filter((code): code is string => code !== undefined)
}

/** Someone with an email address who has sent a proposal bound to SAMPLE_PHONE. Returns its invitation token. */
async function proposal(request: APIRequestContext): Promise<string> {
  const email = `opt-in-${randomUUID().slice(0, 8)}@example.test`
  const code = await codeFrom(
    email,
    'sign-in',
    () => request.post(`${API}/v1/auth/codes`, { data: { identifier: email } }),
    screenshotsLog,
  )
  const session = await request.post(`${API}/v1/auth/sessions`, {
    data: { identifier: email, code, delivery: 'TOKEN', language: 'en' },
  })
  const { token } = (await session.json()) as { token: string }
  const headers = { Authorization: `Bearer ${token}` }
  await request.patch(`${API}/v1/me`, {
    headers,
    data: { display_name: 'Ana Ruiz', adult_confirmed: true },
  })
  const draft = (await (
    await request.post(`${API}/v1/exchanges`, { headers, data: { timezone: 'America/Chicago' } })
  ).json()) as { id: string; version: number }
  const repair = randomUUID()
  const sent = await request.post(`${API}/v1/exchanges/${draft.id}/revisions`, {
    headers,
    data: {
      expected_version: draft.version,
      consent: { language: 'en', version: 'draft-1' },
      invitation: { bound_to: SAMPLE_PHONE },
      terms: {
        party_a_name: 'Ana Ruiz',
        party_b_name: 'Sam Lee',
        terms: 'Repair the back fence.',
        contributions: [
          {
            id: repair,
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
  })
  expect(sent.status(), await sent.text()).toBe(200)
  return ((await sent.json()) as { invitation_token: string }).invitation_token
}

/** Writes a screenshot as WebP, encoded by the browser. */
async function save(browser: Browser, png: Buffer, screen: Screen, language: string) {
  const encoder = await browser.newPage()
  const base64 = await encoder.evaluate(async (data: string) => {
    const blob = await (await fetch(`data:image/png;base64,${data}`)).blob()
    const bitmap = await createImageBitmap(blob)
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
    canvas.getContext('2d')!.drawImage(bitmap, 0, 0)
    const webp = await canvas.convertToBlob({ type: 'image/webp', quality: 0.9 })
    const bytes = new Uint8Array(await webp.arrayBuffer())
    let text = ''
    for (const byte of bytes) text += String.fromCharCode(byte)
    return btoa(text)
  }, png.toString('base64'))
  await encoder.close()
  const file = resolve(webRoot, 'public', screenshotPath(screen, language, 'en').slice(1))
  writeFileSync(file, Buffer.from(base64, 'base64'))
}

/** Deletes the account signed in on `page`, with a code by text, freeing the number for the next run. */
async function deleteAccount(page: Page) {
  const headers = { Origin: webOrigin }
  const code = await phoneCodeFrom('delete', () =>
    page.request.post('/v1/me/deletion/codes', { headers, data: { channel: 'PHONE' } }),
  )
  const done = await page.request.post('/v1/me/deletion', {
    headers,
    data: { channel: 'PHONE', code },
  })
  expect(done.ok(), await done.text()).toBe(true)
}

for (const [language, locale, w] of [
  ['en', 'en-US', en],
  ['es', 'es-ES', es],
] as [string, string, Wording][]) {
  test(`the opt-in screens in ${language}`, async ({ browser, playwright }) => {
    const request = await playwright.request.newContext()
    const token = await proposal(request)
    await request.dispose()

    const context = await browser.newContext({
      baseURL: webOrigin,
      locale,
      timezoneId: 'America/Chicago',
      viewport: { width: SCREENSHOT_WIDTH, height: 844 },
      deviceScaleFactor: 2,
      isMobile: true,
      hasTouch: true,
      colorScheme: 'light',
    })
    const page = await context.newPage()

    // 1. The sign-in form, offering phone numbers, with a number entered.
    await page.goto('/')
    await expect(page.getByText(fill(w.signIn.identifierHintCountries, { codes: '+1' }))).toBeVisible()
    await expect(page.getByText(w.privacy.sms)).toBeVisible()
    await page.getByLabel(w.signIn.identifierLabel, { exact: true }).fill(SAMPLE_PHONE)
    await save(browser, await page.screenshot(), 'signIn', language)

    // 2. After asking for a code.
    const code = await phoneCodeFrom('sign-in', () =>
      page.getByRole('button', { name: w.signIn.sendCode, exact: true }).click(),
    )
    await expect(page.getByText(fill(w.signIn.codeSent, { identifier: SAMPLE_PHONE }))).toBeVisible()
    await page.mouse.click(1, 1)
    await save(browser, await page.screenshot(), 'codeSent', language)

    await page.getByLabel(w.signIn.codeLabel).fill(code)
    await page.getByRole('button', { name: w.signIn.submit, exact: true }).click()
    await expect(page.getByRole('heading', { name: w.profile.firstTitle })).toBeVisible()
    await page.getByLabel(w.profile.nameLabel, { exact: true }).fill('Sam Lee')
    await page.getByLabel(w.profile.adultLabel).check()
    await page.getByRole('button', { name: w.profile.continue, exact: true }).click()
    await expect(page.getByRole('heading', { name: w.home.title, level: 1 })).toBeVisible()

    // The proposal sent to this number, taken up.
    const claimed = await page.request.post('/v1/invitations/claim', {
      headers: { Origin: webOrigin },
      data: { token },
    })
    expect(claimed.ok(), await claimed.text()).toBe(true)
    const { id } = (await claimed.json()) as { id: string }
    await page.goto(`/exchanges/${id}`)

    // 3. "Text updates", the box not yet ticked.
    const control = page.locator('section.sms-updates')
    const box = control.getByRole('checkbox')
    await expect(box).not.toBeChecked()
    await control.scrollIntoViewIfNeeded()
    await save(browser, await control.screenshot(), 'textUpdates', language)

    // 4. Ticked and saved.
    await box.check()
    await control.getByRole('button', { name: w.smsUpdates.save, exact: true }).click()
    const on = fill(w.smsUpdates.on, { phone: `+1 •••-•••-${SAMPLE_PHONE.slice(-4)}` })
    await expect(control.getByText(on)).toBeVisible()
    await control.getByRole('button', { name: w.smsUpdates.save, exact: true }).blur()
    await save(browser, await control.screenshot(), 'confirmation', language)

    await deleteAccount(page)
    await context.close()
  })
}
