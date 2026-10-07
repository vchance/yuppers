import { randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { expect, test, type Browser, type Locator, type Page } from '@playwright/test'
import type { Wording } from '@yuppers/shared'

import {
  SAMPLE_PHONE,
  SAMPLE_PHONE_TO_ADD,
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
 * Takes the pictures of the mobile app on the page on how people opt in to
 * texts, in each language, as a person on an iPhone goes through the steps
 * of each form that texts them in the app:
 *
 *   1. the sign-in screen with a phone number entered, the box beside it not
 *      yet ticked and "Send code" waiting for it;
 *   2. the same with the box ticked and "Send code" enabled;
 *   3. the screen after asking for a code;
 *   4. "Text updates" on an agreement, the box not yet ticked;
 *   5. the same once ticked and saved, with the confirmation;
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
 * Each is the whole screen, with the app's navigation bar, as the phone
 * shows it below its status bar. The screens are the app's own, run in the
 * browser harness; nothing of the browser is in the pictures.
 *
 * The person signs in with SAMPLE_PHONE, a number reserved for fiction,
 * whose code is read back from the API's log; someone else, Ana, who signs
 * in with an email address, has sent them a proposal through the API, which
 * they open from its link. Ana adds SAMPLE_PHONE_TO_ADD, another number
 * reserved for fiction, on the same agreement, and stops once its code is
 * sent (6 to 8). The person who signed in by phone then asks for a code to
 * delete their account (9 to 11), and the account is deleted with that
 * code, so the next run starts the same way.
 *
 * The service sends one number at most five sign-in codes an hour, and a
 * code that confirms a number being added is one of them
 * (`AuthRules::codes_per_hour` in the backend, which no setting changes);
 * deletion codes are counted against the account instead. With the number
 * added apart from the number that signs in, each is sent one code per
 * language: two a run, and four for this run and `npm run screenshots:sms`
 * together within the hour.
 *
 * Each picture is written as WebP, encoded by the browser itself.
 */

/** The screen the pictures are taken at: an iPhone's, at twice the density. */
const SCREEN = { width: SCREENSHOT_WIDTH, height: 844 }

/**
 * The code for a phone number, read back from the log: with
 * SMS_CODE_DELIVERY=log the service writes it where Twilio Verify would
 * text it, the number masked.
 */
async function phoneCodeFrom(
  number: string,
  purpose: 'sign-in' | 'delete',
  send: () => Promise<unknown>,
) {
  const masked = `+1••••••••${number.slice(-2)}`
  const before = phoneCodes(masked, purpose).length
  await send()
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    const codes = phoneCodes(masked, purpose)
    if (codes.length > before) return codes[codes.length - 1]
    await new Promise((settle) => setTimeout(settle, 100))
  }
  throw new Error(`no code for ${masked} in ${screenshotsLog}`)
}

function phoneCodes(masked: string, purpose: 'sign-in' | 'delete'): string[] {
  const log = readFileSync(screenshotsLog, 'utf8')
  const wanted = new RegExp(`purpose="?${purpose === 'delete' ? 'delete-account' : 'sign-in'}"?`)
  return log
    .split('\n')
    .filter((line) => line.includes('one-time code (development delivery)') && line.includes(masked))
    .filter((line) => wanted.test(line))
    .map((line) => /code="?(\d{6})/.exec(line)?.[1])
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

/**
 * Someone with an email address who has sent a proposal bound to
 * SAMPLE_PHONE, in `language`. Returns their address, the exchange and its
 * invitation token.
 */
async function proposal(language: string): Promise<{ email: string; id: string; token: string }> {
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
  await call('PATCH', '/v1/me', { display_name: 'Ana Ruiz', adult_confirmed: true, language }, token)
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
  return { email, id: draft.id, token: sent.invitation_token }
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

/** Deletes the account signed in on `page` with the code texted for it, freeing the number for the next run. */
async function deleteAccount(page: Page, code: string) {
  const token = await page.evaluate(() => window.sessionStorage.getItem('yuppers.harness.session'))
  if (!token) throw new Error('no session in the harness')
  await call('POST', '/v1/me/deletion', { channel: 'PHONE', code }, token)
}

/** Scrolls the screen so that `heading` is at its top, with a little room above for the card's own edge. */
async function toTop(heading: Locator) {
  await heading.evaluate((element) => element.scrollIntoView({ block: 'start' }))
  await heading.evaluate((element) => {
    let scroller = element.parentElement
    while (scroller && scroller.scrollHeight <= scroller.clientHeight) scroller = scroller.parentElement
    scroller?.scrollBy(0, -24)
  })
}

for (const [language, locale, w] of [
  ['en', 'en-US', en],
  ['es', 'es-ES', es],
] as [string, string, Wording][]) {
  test(`the opt-in screens of the mobile app in ${language}`, async ({ browser }) => {
    const { email, id: sent, token } = await proposal(language)

    /** An iPhone of its own, for each person. */
    const iPhone = () =>
      browser.newContext({
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
    const context = await iPhone()
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
    const code = await phoneCodeFrom(SAMPLE_PHONE, 'sign-in', () => send.click())
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
    await toTop(heading)
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

    // 6. Ana, with no number on her account, signs in with her email
    // address and adds one in "Text updates" on the agreement she sent: the
    // number entered, the box beside it unticked, and "Text me a code"
    // waiting for it.
    const anaContext = await iPhone()
    const ana = await anaContext.newPage()
    await ana.goto('/')
    await expect(shown(ana.getByRole('heading', { name: w.signIn.title, level: 1 }))).toBeVisible()
    await shown(ana.getByLabel(w.signIn.identifierLabel)).fill(email)
    const anaCode = await codeFrom(
      email,
      'sign-in',
      () => shown(ana.getByRole('button', { name: w.signIn.sendCode, exact: true })).click(),
      screenshotsLog,
    )
    await shown(ana.getByLabel(w.signIn.codeLabel)).fill(anaCode)
    await shown(ana.getByRole('button', { name: w.signIn.submit, exact: true })).click()
    await expect(shown(ana.getByRole('heading', { name: w.home.title, level: 1 }))).toBeVisible()
    await ana.goto(`/exchanges/${sent}`)
    const adding = shown(ana.getByRole('heading', { name: w.smsUpdates.heading, level: 2 }))
    await expect(shown(ana.getByText(w.smsUpdates.addPhoneIntro, { exact: true }))).toBeVisible()
    const number = shown(ana.getByLabel(w.smsUpdates.phoneLabel))
    await number.fill(SAMPLE_PHONE_TO_ADD)
    await number.blur()
    const numberBox = shown(ana.getByRole('checkbox', { name: w.smsCode.verifyNumber, exact: true }))
    const textMe = shown(ana.getByRole('button', { name: w.smsUpdates.sendCode, exact: true }))
    await expect(numberBox).not.toBeChecked()
    await expect(textMe).toBeDisabled()
    await expect(shown(ana.getByText(w.smsCode.tickToSend, { exact: true }))).toBeVisible()
    await toTop(adding)
    await save(browser, await ana.screenshot(), 'confirmNumber', language)

    // 7. The box ticked: "Text me a code" enabled.
    await numberBox.click()
    await expect(numberBox).toBeChecked()
    await expect(textMe).toBeEnabled()
    await numberBox.blur()
    await save(browser, await ana.screenshot(), 'confirmNumberTicked', language)

    // 8. After asking for the code. The number is left unconfirmed.
    await phoneCodeFrom(SAMPLE_PHONE_TO_ADD, 'sign-in', () => textMe.click())
    const toAdd = `+1 •••-•••-${SAMPLE_PHONE_TO_ADD.slice(-4)}`
    await expect(
      shown(ana.getByText(fill(w.smsUpdates.codeSent, { phone: toAdd }), { exact: true })),
    ).toBeVisible()
    await toTop(adding)
    await ana.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
    await save(browser, await ana.screenshot(), 'confirmNumberCodeSent', language)
    await anaContext.close()

    // 9. The person who signed in by phone deletes their account: the code
    // to go to the number, the box beside it unticked, and "Send code and
    // continue" waiting for it.
    await page.goto('/account')
    await shown(page.getByRole('button', { name: w.deletion.open, exact: true })).click()
    await expect(
      shown(page.getByText(fill(w.deletion.codeIntro, { identifier: SAMPLE_PHONE }), { exact: true })),
    ).toBeVisible()
    const deletionBox = shown(page.getByRole('checkbox', { name: w.smsCode.deleteAccount, exact: true }))
    const sendDeletion = shown(page.getByRole('button', { name: w.deletion.sendCode, exact: true }))
    await expect(deletionBox).not.toBeChecked()
    await expect(sendDeletion).toBeDisabled()
    await expect(shown(page.getByText(w.smsCode.tickToSend, { exact: true }))).toBeVisible()
    // From where the code goes to the button, the box in the middle.
    await deletionBox.evaluate((element) => element.scrollIntoView({ block: 'center' }))
    await save(browser, await page.screenshot(), 'deleteAccount', language)

    // 10. The box ticked: the button enabled.
    await deletionBox.click()
    await expect(deletionBox).toBeChecked()
    await expect(sendDeletion).toBeEnabled()
    await deletionBox.blur()
    await save(browser, await page.screenshot(), 'deleteAccountTicked', language)

    // 11. After asking for the code. Nothing is deleted on screen; the
    // account is deleted with this code, through the API, once the picture
    // is taken.
    const deletionCode = await phoneCodeFrom(SAMPLE_PHONE, 'delete', () => sendDeletion.click())
    const deletionSent = shown(
      page.getByText(fill(w.deletion.codeSent, { identifier: SAMPLE_PHONE }), { exact: true }),
    )
    await expect(deletionSent).toBeVisible()
    await deletionSent.evaluate((element) => element.scrollIntoView({ block: 'center' }))
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
    await save(browser, await page.screenshot(), 'deleteAccountCodeSent', language)

    await deleteAccount(page, deletionCode)
    await context.close()
  })
}
