import { spawn } from 'node:child_process'
import { closeSync, openSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  CODES_ANCHOR,
  CODE_FORMS,
  FORMS,
  PART_ANCHORS,
  SCREENSHOTS,
  UPDATE_FORMS,
  VERIFY_DELETION_SAMPLE,
  VERIFY_SAMPLE,
  codesAnchor,
  formAnchor,
  stepAnchor,
  type Form,
  type StepName,
} from '../build/sms-opt-in'
import { apiEnvironment, apiLog, port, repoRoot, webRoot, workerBinary } from './support/env'
import { codeFrom } from './support/codes'
import { expect, test } from './support/fixtures'
import { agree, move, type ItemSpec } from './support/flows'
import { american, codesTo, number, textsTo, waitFor } from './support/texts'
import { en, fill } from './support/wording'

/*
 * Text updates for an agreement ("Yuppers.app agreement updates"): a party
 * adds a number with a code by text, ticks the box beside the consent
 * wording, sees the confirmation, and once the other party marks something
 * delivered, the worker texts them; and the page the carriers' reviewers
 * are given, as the service serves it.
 *
 * The API here writes texts to its log (`SMS_DELIVERY=log`), and codes for
 * phone numbers (`SMS_CODE_DELIVERY=log`); the worker is
 * started for the one pass that sends them, and writes them to a log of its
 * own.
 */

const BIKE = 'A blue bicycle'
const ITEMS: ItemSpec[] = [
  { from: 'me', kind: 'ITEM', description: BIKE },
  { from: 'them', kind: 'MONEY', description: 'Payment', amount: '120' },
]

test('a party adds a number, turns on text updates, and is texted when the agreement changes', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  const id = await agree(ana, bruno, ITEMS)
  const w = en.smsUpdates
  const page = bruno.page
  const phone = number()

  // No number on the account yet: one to add, checked with a code by text.
  const control = page.getByRole('region', { name: w.heading, exact: true })
  await expect(control.getByText(w.addPhoneIntro)).toBeVisible()
  // Typed as people in the US write it, without +1.
  await control.getByLabel(w.phoneLabel).fill(american(phone))
  // The code goes by text only once the box beside the number is ticked.
  const sendCode = control.getByRole('button', { name: w.sendCode, exact: true })
  await expect(sendCode).toBeDisabled()
  const codeBox = control.getByRole('checkbox', { name: en.smsCode.verifyNumber, exact: true })
  await expect(codeBox).not.toBeChecked()
  await codeBox.check()
  const before = codesTo(phone, apiLog).length
  // With it, a code by email to the account's own address: adding a number
  // takes a proof of it.
  const proofCode = await codeFrom(bruno.email, 'sign-in', () => sendCode.click())
  await expect(control.getByText(fill(w.codeSent, { phone: `(•••) •••-${phone.slice(-4)}` }))).toBeVisible()
  await expect(control.getByText(fill(w.proofCodeSent, { email: bruno.email }))).toBeVisible()
  // Twilio Verify would text it; here the log has it. No text of the
  // service's own carries it.
  const code = await waitFor(() => codesTo(phone, apiLog)[before], 'the code for the number')
  expect(textsTo(phone, apiLog)).toEqual([])
  await control.getByLabel(w.codeLabel).fill(code)
  await control.getByLabel(w.proofCodeLabel).fill(proofCode)
  await control.getByRole('button', { name: w.addPhone, exact: true }).click()

  // The box, beside the consent wording word for word, not yet ticked.
  const box = control.getByRole('checkbox', { name: w.consent })
  await expect(box).not.toBeChecked()
  await expect(control.getByRole('link', { name: 'https://yuppers.app/terms' })).toHaveAttribute(
    'href',
    'https://yuppers.app/terms',
  )
  await expect(control.getByRole('link', { name: 'https://yuppers.app/privacy' })).toHaveAttribute(
    'href',
    'https://yuppers.app/privacy',
  )
  await box.check()
  await control.getByRole('button', { name: w.save, exact: true }).click()
  const confirmation = fill(w.on, { phone: `(•••) •••-${phone.slice(-4)}` })
  await expect(control.getByText(confirmation)).toBeVisible()
  expect(confirmation).toBe(
    `Text updates are on for this agreement. You’ll get one text per status change at (•••) •••-${phone.slice(-4)}. Reply STOP to opt out.`,
  )
  // Still on after a reload.
  await page.reload()
  await expect(page.getByRole('region', { name: w.heading, exact: true }).getByRole('checkbox')).toBeChecked()

  // Ana marks the bicycle delivered: a status change Bruno turned updates on for.
  await move(ana.page, BIKE, en.exchange.moves.CLAIM)

  // The worker sends what was queued: the confirmation, then the update.
  const workerLog = resolve(webRoot, `e2e/.output/worker-${phone.slice(-6)}.log`)
  const env = apiEnvironment(port)
  const output = openSync(workerLog, 'w')
  const worker = spawn(workerBinary, [], {
    cwd: repoRoot,
    env: { ...process.env, ...env, WEB_DIR: '' },
    stdio: ['ignore', output, output],
  })
  try {
    const link = `http://127.0.0.1:${port}/exchanges/${id}`
    const update = `Yuppers.app: an agreement you turned on updates for has changed. See it: ${link}. Reply STOP to opt out.`
    const texts = await waitFor(() => {
      const found = textsTo(phone, workerLog)
      return found.includes(update) ? found : undefined
    }, `the update text in ${workerLog}`)
    expect(texts).toEqual([en.sms.optInConfirmation, update])
  } finally {
    worker.kill('SIGTERM')
    closeSync(output)
  }
})

test('the page on how people opt in shows each form on the website and in the app, and their pictures, under the same policy', async ({
  person,
}) => {
  const reader = await person('Reviewer', { javaScriptEnabled: false, viewport: { width: 390, height: 844 } })
  const { page } = reader
  const wording = JSON.parse(
    readFileSync(resolve(repoRoot, 'packages/shared/wording/sms-opt-in/en.json'), 'utf8'),
  ) as {
    title: string
    contentsLabel: string
    websiteHeading: string
    mobileHeading: string
    mobileRelease: string
    codesHeading: string
    forms: Record<Form, { heading: string }>
    steps: Record<string, { title: string }>
    mobileSteps: Record<string, { title: string; caption: string }>
  }
  const steps = (forms: readonly Form[]) => forms.reduce((count, form) => count + FORMS[form].length, 0)
  const screens = Object.keys(SCREENSHOTS).length

  // Reached from the sections on texts of the terms and the privacy policy.
  for (const document of ['terms', 'privacy']) {
    await page.goto(`/${document}#text-messages`)
    const texts = page.locator('section:has(> h2#text-messages)')
    await texts.locator('a[href="/sms-opt-in"]').click()
    await expect(page).toHaveURL(/\/sms-opt-in$/)
  }

  for (const [address, language] of [
    ['/sms-opt-in', 'en'],
    ['/es/sms-opt-in', 'es'],
  ]) {
    const response = await page.goto(address)
    expect(response?.status(), address).toBe(200)
    expect(response?.headers()['content-security-policy']).toBe(
      "default-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; script-src 'self' 'sha256-KxR9MhTq1F37YaceB87TcfvvGJe+U71nuBxrnuq+JCQ='",
    )
    await expect(page.locator('html')).toHaveAttribute('lang', language)
    // Agreement updates and HELP and STOP in each part, each under its own
    // anchor, and every step of each; then the forms that text a one-time
    // code through Twilio Verify, apart, for each part.
    const codes = page.locator(`section.part:has(> h2#${CODES_ANCHOR})`)
    for (const surface of ['web', 'mobile'] as const) {
      const section = page.locator(`section.part:has(> h2#${PART_ANCHORS[surface]})`)
      await expect(section.locator('section.form > h3')).toHaveCount(UPDATE_FORMS.length)
      for (const form of UPDATE_FORMS) {
        await expect(section.locator(`h3#${formAnchor(form, surface)}`)).toHaveCount(1)
      }
      await expect(section.locator('section.step > h4')).toHaveCount(steps(UPDATE_FORMS))
      const among = codes.locator(`section.surface:has(> h3#${codesAnchor(surface)})`)
      await expect(among.locator('section.form > h4')).toHaveCount(CODE_FORMS.length)
      for (const form of CODE_FORMS) {
        await expect(among.locator(`h4#${formAnchor(form, surface)}`)).toHaveCount(1)
      }
      await expect(among.locator('section.step > h5')).toHaveCount(steps(CODE_FORMS))
      // After each code is sent, the message Twilio Verify sends; the
      // deletion service's names what its code is for.
      await expect(among.getByText(VERIFY_SAMPLE, { exact: true })).toHaveCount(CODE_FORMS.length - 1)
      const deletion = among.locator(`section.form:has(> h4#${formAnchor('deleteAccount', surface)})`)
      await expect(deletion.getByText(VERIFY_DELETION_SAMPLE, { exact: true })).toHaveCount(1)
    }
    // Every picture is there, from this origin, served, and drawn.
    const images = page.locator('section.step img')
    await expect(images).toHaveCount(2 * screens)
    // The app's pictures: its updates in its part, its code forms among the codes.
    await expect(page.locator('section.step img[src^="/sms-opt-in/mobile/"]')).toHaveCount(screens)
    await expect(page.locator('section.part:has(> h2#mobile-app) img')).toHaveCount(
      UPDATE_FORMS.reduce((count, form) => count + (FORMS[form] as readonly string[]).filter((step) => step in SCREENSHOTS).length, 0),
    )
    for (const image of await images.all()) {
      await image.scrollIntoViewIfNeeded()
      const src = (await image.getAttribute('src'))!
      expect(src).toMatch(/^\/sms-opt-in\//)
      const served = await page.request.get(src)
      expect(served.status(), src).toBe(200)
      expect(served.headers()['content-type'], src).toBe('image/webp')
      await expect
        .poll(() => image.evaluate((element: HTMLImageElement) => element.complete && element.naturalWidth))
        .toBeGreaterThan(300)
    }
  }
  await page.goto('/sms-opt-in')
  await expect(page.getByRole('heading', { name: wording.title, level: 1 })).toBeVisible()
  await expect(page.getByRole('heading', { name: wording.websiteHeading, level: 2 })).toBeVisible()
  await expect(page.getByRole('heading', { name: wording.codesHeading, level: 2 })).toBeVisible()
  for (const [step, { title }] of Object.entries(wording.steps)) {
    await expect(page.locator(`#${stepAnchor(step as StepName, 'web')}`)).toHaveText(title)
  }
  await expect(page.getByText(en.smsUpdates.consent, { exact: true }).first()).toBeVisible()
  for (const label of [en.smsCode.signIn, en.smsCode.verifyNumber, en.smsCode.deleteAccount]) {
    await expect(page.getByText(label, { exact: true }).first()).toBeVisible()
  }
  await expect(page.getByText(en.sms.optInConfirmation, { exact: true }).first()).toBeVisible()

  // The contents list leads to each part and each form, at its own address.
  const contents = page.getByRole('navigation', { name: wording.contentsLabel })
  await expect(contents.getByRole('link')).toHaveCount(
    2 + 2 * UPDATE_FORMS.length + 1 + 2 + 2 * CODE_FORMS.length,
  )
  await expect(contents.getByRole('link', { name: wording.websiteHeading })).toHaveAttribute('href', '#website')
  await contents.getByRole('link', { name: wording.mobileHeading }).click()
  await expect(page).toHaveURL(/\/sms-opt-in#mobile-app$/)
  const mobile = page.getByRole('heading', { name: wording.mobileHeading, level: 2 })
  await expect(mobile).toBeInViewport()
  await expect(page.getByText(wording.mobileRelease, { exact: true })).toBeVisible()
  for (const [step, { title, caption }] of Object.entries(wording.mobileSteps)) {
    const heading = page.locator(`#${stepAnchor(step as StepName, 'mobile')}`)
    await expect(heading).toHaveText(title)
    await expect(heading.locator('xpath=following-sibling::p[1]')).toHaveText(caption)
  }
  await contents
    .getByRole('link', { name: wording.forms.confirmNumber.heading })
    .nth(1)
    .click()
  await expect(page).toHaveURL(/\/sms-opt-in#mobile-app-confirm-number$/)
  await expect(page.locator('h4#mobile-app-confirm-number')).toBeInViewport()
  // A link straight to a part, or to one form, opens there: the addresses
  // the campaign gives, and the ones the code forms had before they moved.
  await page.goto('/es/sms-opt-in#mobile-app')
  await expect(page.locator('h2#mobile-app')).toBeInViewport()
  for (const anchor of ['website-agreement-updates', 'mobile-app-agreement-updates']) {
    await page.goto(`/sms-opt-in#${anchor}`)
    await expect(page.locator(`h3#${anchor}`)).toBeInViewport()
  }
  await page.goto('/es/sms-opt-in#website-delete-account')
  await expect(page.locator('h4#website-delete-account')).toBeInViewport()
})
