import { spawn } from 'node:child_process'
import { closeSync, openSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { apiEnvironment, apiLog, port, repoRoot, webRoot, workerBinary } from './support/env'
import { expect, test } from './support/fixtures'
import { agree, move, type ItemSpec } from './support/flows'
import { codeIn, number, textsTo, waitFor } from './support/texts'
import { en, fill } from './support/wording'

/*
 * Text updates for an agreement ("Yuppers.app agreement updates"): a party
 * adds a number with a code by text, ticks the box beside the consent
 * wording, sees the confirmation, and once the other party marks something
 * delivered, the worker texts them; and the page the carriers' reviewers
 * are given, as the service serves it.
 *
 * The API here writes texts to its log (`SMS_DELIVERY=log`); the worker is
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
  await control.getByLabel(w.phoneLabel).fill(phone)
  // The code goes by text only once the box beside the number is ticked.
  const sendCode = control.getByRole('button', { name: w.sendCode, exact: true })
  await expect(sendCode).toBeDisabled()
  const codeBox = control.getByRole('checkbox', { name: en.smsCode.verifyNumber, exact: true })
  await expect(codeBox).not.toBeChecked()
  await codeBox.check()
  const before = textsTo(phone, apiLog).length
  await sendCode.click()
  await expect(control.getByText(fill(w.codeSent, { phone: `+1 •••-•••-${phone.slice(-4)}` }))).toBeVisible()
  const codeText = await waitFor(() => textsTo(phone, apiLog)[before], 'the code by text')
  const code = codeIn(codeText)
  // The text says what the code is for: confirming the number, not signing in.
  expect(codeText).toBe(
    `Yuppers.app: ${code} is your code to confirm this phone number. Do not share it with anyone.`,
  )
  expect(codeText).toBe(fill(en.sms.verifyNumber, { code }))
  await control.getByLabel(w.codeLabel).fill(code)
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
  const confirmation = fill(w.on, { phone: `+1 •••-•••-${phone.slice(-4)}` })
  await expect(control.getByText(confirmation)).toBeVisible()
  expect(confirmation).toBe(
    `Text updates are on for this agreement. You’ll get one text per status change at +1 •••-•••-${phone.slice(-4)}. Reply STOP to opt out.`,
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

test('the page on how people opt in shows its steps on the website and in the app, and their pictures, under the same policy', async ({
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
    steps: Record<string, { title: string }>
    mobileSteps: Record<string, { title: string; caption: string }>
  }

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
      "default-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    )
    await expect(page.locator('html')).toHaveAttribute('lang', language)
    // Seven steps on the website, five in the app.
    await expect(page.locator('section.part:has(> h2#website) section.step > h3')).toHaveCount(7)
    await expect(page.locator('section.part:has(> h2#mobile-app) section.step > h3')).toHaveCount(5)
    // Every picture is there, from this origin, served, and drawn.
    const images = page.locator('section.step img')
    await expect(images).toHaveCount(10)
    await expect(page.locator('section.part:has(> h2#mobile-app) img[src^="/sms-opt-in/mobile/"]')).toHaveCount(5)
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
  for (const step of Object.values(wording.steps)) {
    await expect(page.getByRole('heading', { name: step.title, level: 3, exact: true })).toBeVisible()
  }
  await expect(page.getByText(en.smsUpdates.consent, { exact: true }).first()).toBeVisible()
  await expect(page.getByText(en.smsCode.signIn, { exact: true }).first()).toBeVisible()
  await expect(page.getByText(en.sms.optInConfirmation, { exact: true })).toBeVisible()

  // The contents list leads to the mobile app's part, at its own address.
  const contents = page.getByRole('navigation', { name: wording.contentsLabel })
  await expect(contents.getByRole('link', { name: wording.websiteHeading })).toHaveAttribute('href', '#website')
  await contents.getByRole('link', { name: wording.mobileHeading }).click()
  await expect(page).toHaveURL(/\/sms-opt-in#mobile-app$/)
  const mobile = page.getByRole('heading', { name: wording.mobileHeading, level: 2 })
  await expect(mobile).toBeInViewport()
  await expect(page.getByText(wording.mobileRelease, { exact: true })).toBeVisible()
  for (const step of Object.values(wording.mobileSteps)) {
    await expect(page.getByRole('heading', { name: step.title, level: 3, exact: true })).toBeVisible()
    await expect(page.getByText(step.caption, { exact: true })).toBeVisible()
  }
  // A link straight to the app's part opens there.
  await page.goto('/es/sms-opt-in#mobile-app')
  await expect(page.locator('h2#mobile-app')).toBeInViewport()
})
