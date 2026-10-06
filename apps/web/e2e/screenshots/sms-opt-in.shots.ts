import { randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { expect, test, type APIRequestContext, type Browser, type Page } from '@playwright/test'
import type { Wording } from '@yuppers/shared'

import {
  SAMPLE_PHONE,
  SAMPLE_PHONE_TO_ADD,
  SCREENSHOT_WIDTH,
  screenshotPath,
  type Screen,
} from '../../build/sms-opt-in'
import { codeFrom } from '../support/codes'
import { webRoot } from '../support/env'
import { en, es, fill } from '../support/wording'
import { apiPort, screenshotsLog, webOrigin } from './playwright.config'

/*
 * Takes the pictures of the page on how people opt in to texts, in each
 * language, as a person on a phone goes through the steps of each form that
 * texts them:
 *
 *   1. the sign-in form with a phone number entered, the box beside it not
 *      yet ticked and "Send code" waiting for it;
 *   2. the same with the box ticked and "Send code" enabled;
 *   3. the form after asking for a code;
 *   4. "Text updates" on an agreement, the box not yet ticked;
 *   5. the same once ticked and saved;
 *   6. "Text updates" on an account with no number: a number entered to be
 *      added, the box beside it not yet ticked and "Text me a code" waiting
 *      for it;
 *   7. the same with the box ticked;
 *   8. after asking for the code that confirms the number;
 *   9. deleting the account, with the code to go to its phone number, the
 *      box beside it not yet ticked and "Send code and continue" waiting;
 *  10. the same with the box ticked;
 *  11. after asking for the deletion code.
 *
 * The person signs in with SAMPLE_PHONE, a number reserved for fiction,
 * whose code is read back from the API's log; someone else, Ana, who signs
 * in with an email address, has sent them a proposal through the API. Ana
 * adds SAMPLE_PHONE_TO_ADD, another number reserved for fiction, on the
 * same agreement, and stops once its code is sent (6 to 8). The person who
 * signed in by phone then asks for a code to delete their account (9 to
 * 11), and the account is deleted with that code, so the next run starts
 * the same way.
 *
 * The service sends one number at most five sign-in codes an hour, and a
 * code that confirms a number being added is one of them
 * (`AuthRules::codes_per_hour` in the backend, which no setting changes);
 * deletion codes are counted against the account instead. With the number
 * added apart from the number that signs in, each is sent one code per
 * language: two a run, and four for this run and `npm run
 * screenshots:sms:mobile` together within the hour.
 *
 * Each picture is written as WebP, encoded by the browser itself.
 */

const API = `http://127.0.0.1:${apiPort}`

/** The text message a code went out in, read back from the log, for a phone number. */
function phoneCodeFrom(phone: string, purpose: 'sign-in' | 'delete', send: () => Promise<unknown>) {
  const masked = `+1••••••••${phone.slice(-2)}`
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
    .map((line) => /text="?Yuppers\.app: (\d{6})/.exec(line)?.[1])
    .filter((code): code is string => code !== undefined)
}

/**
 * Someone with an email address who has sent a proposal bound to
 * SAMPLE_PHONE, in `language`. Returns their address, the exchange and its
 * invitation token.
 */
async function proposal(
  request: APIRequestContext,
  language: string,
): Promise<{ email: string; id: string; token: string }> {
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
    data: { display_name: 'Ana Ruiz', adult_confirmed: true, language },
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
  const { invitation_token } = (await sent.json()) as { invitation_token: string }
  return { email, id: draft.id, token: invitation_token }
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

/** Deletes the account signed in on `page` with the code texted for it, freeing the number for the next run. */
async function deleteAccount(page: Page, code: string) {
  const done = await page.request.post('/v1/me/deletion', {
    headers: { Origin: webOrigin },
    data: { channel: 'PHONE', code },
  })
  expect(done.ok(), await done.text()).toBe(true)
}

/** Signs in on `page` with an email address, as Ana does on a phone of her own, and opens exchange `id`. */
async function signInByEmail(page: Page, w: Wording, email: string, id: string) {
  await page.goto('/')
  await page.getByLabel(w.signIn.identifierLabel, { exact: true }).fill(email)
  const code = await codeFrom(
    email,
    'sign-in',
    () => page.getByRole('button', { name: w.signIn.sendCode, exact: true }).click(),
    screenshotsLog,
  )
  await page.getByLabel(w.signIn.codeLabel).fill(code)
  await page.getByRole('button', { name: w.signIn.submit, exact: true }).click()
  await expect(page.getByRole('heading', { name: w.home.title, level: 1 })).toBeVisible()
  await page.goto(`/exchanges/${id}`)
}

for (const [language, locale, w] of [
  ['en', 'en-US', en],
  ['es', 'es-ES', es],
] as [string, string, Wording][]) {
  test(`the opt-in screens in ${language}`, async ({ browser, playwright }) => {
    const request = await playwright.request.newContext()
    const { email, id: sent, token } = await proposal(request, language)
    await request.dispose()

    /** A phone of its own, for each person. */
    const phone = () =>
      browser.newContext({
        baseURL: webOrigin,
        locale,
        timezoneId: 'America/Chicago',
        viewport: { width: SCREENSHOT_WIDTH, height: 844 },
        deviceScaleFactor: 2,
        isMobile: true,
        hasTouch: true,
        colorScheme: 'light',
      })
    const context = await phone()
    const page = await context.newPage()

    // 1. The sign-in form, offering phone numbers, with a number entered:
    // the consentBox beside it, unticked, and "Send code" waiting for it.
    await page.goto('/')
    await expect(page.getByText(fill(w.signIn.identifierHintCountries, { codes: '+1' }))).toBeVisible()
    await page.getByLabel(w.signIn.identifierLabel, { exact: true }).fill(SAMPLE_PHONE)
    const consentBox = page.getByRole('checkbox', { name: w.smsCode.signIn, exact: true })
    const send = page.getByRole('button', { name: w.signIn.sendCode, exact: true })
    await expect(consentBox).not.toBeChecked()
    await expect(send).toBeDisabled()
    await expect(page.getByText(w.smsCode.tickToSend)).toBeVisible()
    await expect(page.getByRole('link', { name: w.privacy.smsLink })).toBeVisible()
    await page.mouse.click(1, 1)
    await save(browser, await page.screenshot({ fullPage: true }), 'signIn', language)

    // 2. The consentBox ticked: "Send code" enabled.
    await consentBox.check()
    await expect(send).toBeEnabled()
    await page.mouse.click(1, 1)
    await save(browser, await page.screenshot({ fullPage: true }), 'boxTicked', language)

    // 3. After asking for a code.
    const code = await phoneCodeFrom(SAMPLE_PHONE, 'sign-in', () => send.click())
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

    // 4. "Text updates", the box not yet ticked.
    const control = page.locator('section.sms-updates')
    const box = control.getByRole('checkbox')
    await expect(box).not.toBeChecked()
    await control.scrollIntoViewIfNeeded()
    await save(browser, await control.screenshot(), 'textUpdates', language)

    // 5. Ticked and saved.
    await box.check()
    await control.getByRole('button', { name: w.smsUpdates.save, exact: true }).click()
    const on = fill(w.smsUpdates.on, { phone: `+1 •••-•••-${SAMPLE_PHONE.slice(-4)}` })
    await expect(control.getByText(on)).toBeVisible()
    await control.getByRole('button', { name: w.smsUpdates.save, exact: true }).blur()
    await save(browser, await control.screenshot(), 'confirmation', language)

    // 6. Ana, with no number on her account, adds one in "Text updates" on
    // the agreement she sent: the number entered, the box beside it
    // unticked, and "Text me a code" waiting for it.
    const anaContext = await phone()
    const ana = await anaContext.newPage()
    await signInByEmail(ana, w, email, sent)
    const adding = ana.locator('section.sms-updates')
    await expect(adding.getByText(w.smsUpdates.addPhoneIntro)).toBeVisible()
    await adding.getByLabel(w.smsUpdates.phoneLabel).fill(SAMPLE_PHONE_TO_ADD)
    const numberBox = adding.getByRole('checkbox', { name: w.smsCode.verifyNumber, exact: true })
    const textMe = adding.getByRole('button', { name: w.smsUpdates.sendCode, exact: true })
    await expect(numberBox).not.toBeChecked()
    await expect(textMe).toBeDisabled()
    await expect(adding.getByText(w.smsCode.tickToSend)).toBeVisible()
    await ana.mouse.click(1, 1)
    await adding.scrollIntoViewIfNeeded()
    await save(browser, await adding.screenshot(), 'confirmNumber', language)

    // 7. The box ticked: "Text me a code" enabled.
    await numberBox.check()
    await expect(textMe).toBeEnabled()
    await ana.mouse.click(1, 1)
    await save(browser, await adding.screenshot(), 'confirmNumberTicked', language)

    // 8. After asking for the code. The number is left unconfirmed.
    await phoneCodeFrom(SAMPLE_PHONE_TO_ADD, 'sign-in', () => textMe.click())
    const toAdd = `+1 •••-•••-${SAMPLE_PHONE_TO_ADD.slice(-4)}`
    await expect(adding.getByText(fill(w.smsUpdates.codeSent, { phone: toAdd }))).toBeVisible()
    await ana.mouse.click(1, 1)
    await save(browser, await adding.screenshot(), 'confirmNumberCodeSent', language)
    await anaContext.close()

    // 9. The person who signed in by phone deletes their account: the code
    // to go to the number, the box beside it unticked, and "Send code and
    // continue" waiting for it.
    await page.goto('/account')
    const deletion = page.getByRole('region', { name: w.deletion.heading, exact: true })
    await deletion.getByRole('button', { name: w.deletion.open, exact: true }).click()
    await expect(
      deletion.getByText(fill(w.deletion.codeIntro, { identifier: SAMPLE_PHONE })),
    ).toBeVisible()
    const deletionBox = deletion.getByRole('checkbox', { name: w.smsCode.deleteAccount, exact: true })
    const sendDeletion = deletion.getByRole('button', { name: w.deletion.sendCode, exact: true })
    await expect(deletionBox).not.toBeChecked()
    await expect(sendDeletion).toBeDisabled()
    await expect(deletion.getByText(w.smsCode.tickToSend)).toBeVisible()
    await page.mouse.click(1, 1)
    // From where the code goes to the button, the box in the middle.
    await deletionBox.evaluate((element) => element.scrollIntoView({ block: 'center' }))
    await save(browser, await page.screenshot(), 'deleteAccount', language)

    // 10. The box ticked: the button enabled.
    await deletionBox.check()
    await expect(sendDeletion).toBeEnabled()
    await page.mouse.click(1, 1)
    await deletionBox.evaluate((element) => element.scrollIntoView({ block: 'center' }))
    await save(browser, await page.screenshot(), 'deleteAccountTicked', language)

    // 11. After asking for the code. Nothing is deleted on screen; the
    // account is deleted with this code, through the API, once the picture
    // is taken.
    const deletionCode = await phoneCodeFrom(SAMPLE_PHONE, 'delete', () => sendDeletion.click())
    await expect(
      deletion.getByText(fill(w.deletion.codeSent, { identifier: SAMPLE_PHONE })),
    ).toBeVisible()
    await page.mouse.click(1, 1)
    await deletion.evaluate((element) => element.scrollIntoView({ block: 'start' }))
    await save(browser, await page.screenshot(), 'deleteAccountCodeSent', language)

    await deleteAccount(page, deletionCode)
    await context.close()
  })
}
