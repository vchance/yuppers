import { apiLog } from './support/env'
import { expect, test } from './support/fixtures'
import { signIn } from './support/flows'
import { american, codesTo, number, waitFor } from './support/texts'
import { en, fill } from './support/wording'

/*
 * Signing in asks for what the service can send codes to (README, "Signing
 * in"). The API here writes codes to its log, phone codes included, so it
 * offers both; a deployment without text messages is stood in for by
 * answering `GET /v1/meta` in the browser as such a service would.
 */

test('the service offers phone numbers here, and the form asks for US numbers', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana
  const meta = (await (await page.request.get('/v1/meta')).json()) as {
    sign_in_channels: string[]
    sms_country_codes: string[]
  }
  expect(meta.sign_in_channels).toEqual(['email', 'phone'])
  expect(meta.sms_country_codes).toEqual(['+1'])

  await page.goto('/')
  await expect(page.getByLabel(en.signIn.identifierLabel)).toBeVisible()
  // US numbers only, written as they are there: no country code asked for.
  await expect(page.getByText(en.signIn.identifierHintUs)).toBeVisible()
  await signIn(ana)
  await expect(page.getByRole('heading', { name: en.profile.firstTitle })).toBeVisible()
})

test('a phone number gets its code by text only once the box beside it is ticked', async ({ person }) => {
  const sam = await person('Sam')
  const { page } = sam
  const phone = number()
  const asked: unknown[] = []
  page.on('request', (request) => {
    if (request.url().endsWith('/v1/auth/codes')) asked.push(request.postDataJSON())
  })

  // Without the box, as from a client from before it, the service refuses.
  const refused = await page.request.post('/v1/auth/codes', { data: { identifier: phone } })
  expect(refused.status()).toBe(422)
  expect(await refused.json()).toEqual({ code: 'SMS_CONSENT_REQUIRED' })

  await page.goto('/')
  const identifier = page.getByLabel(en.signIn.identifierLabel)
  const box = page.getByRole('checkbox', { name: en.smsCode.signIn, exact: true })
  const send = page.getByRole('button', { name: en.signIn.sendCode, exact: true })
  // An email address: no box, and nothing to wait for.
  await identifier.fill(sam.email)
  await expect(box).toHaveCount(0)
  await expect(send).toBeEnabled()

  // A number, typed as people in the US write it, without +1: the box,
  // unticked, its addresses links to a new tab, and the button waiting for
  // it, saying why. The short line it replaces is gone.
  await identifier.fill(american(phone))
  await expect(box).toBeVisible()
  await expect(box).not.toBeChecked()
  await expect(send).toBeDisabled()
  await expect(send).toHaveAccessibleDescription(en.smsCode.tickToSend)
  for (const address of ['https://yuppers.app/terms', 'https://yuppers.app/privacy']) {
    const link = page.getByRole('link', { name: address, exact: true })
    await expect(link).toHaveAttribute('href', address)
    await expect(link).toHaveAttribute('target', '_blank')
  }
  await expect(page.getByText('Message and data rates may apply. Reply STOP to opt out.')).toHaveCount(0)
  await expect(page.getByRole('link', { name: en.privacy.smsLink })).toBeVisible()
  await send.click({ force: true })
  expect(asked).toEqual([])

  await box.check()
  await expect(send).toBeEnabled()
  await expect(identifier).toHaveValue(american(phone))
  const before = codesTo(phone, apiLog).length
  await send.click()
  // Sent in E.164, and shown the American way.
  await expect(page.getByText(fill(en.signIn.codeSent, { identifier: american(phone) }))).toBeVisible()
  expect(asked).toEqual([{ identifier: phone, sms_consent: { version: expect.any(String), language: 'en' } }])
  // Twilio Verify would text it; here the log has it.
  const code = await waitFor(() => codesTo(phone, apiLog)[before], 'the code for the number')
  await page.getByLabel(en.signIn.codeLabel).fill(code)
  await page.getByRole('button', { name: en.signIn.submit, exact: true }).click()
  await expect(page.getByRole('heading', { name: en.profile.firstTitle })).toBeVisible()
})

test('where the service has no text messages, an email address is asked for, and a phone number stopped', async ({
  person,
}) => {
  const ben = await person('Ben')
  const { page } = ben
  await page.route('**/v1/meta', async (route) => {
    const response = await route.fetch()
    const meta = (await response.json()) as Record<string, unknown>
    await route.fulfill({
      response,
      json: { ...meta, sign_in_channels: ['email'], sms_country_codes: [] },
    })
  })
  const codesAsked: string[] = []
  page.on('request', (request) => {
    if (request.url().endsWith('/v1/auth/codes')) codesAsked.push(request.url())
  })

  await page.goto('/')
  await expect(page.getByText(en.signIn.introEmail)).toBeVisible()
  const email = page.getByLabel(en.signIn.emailLabel, { exact: true })
  await expect(email).toHaveAttribute('type', 'email')
  await expect(page.getByLabel(en.signIn.identifierLabel)).toHaveCount(0)

  await email.fill('+1 202 555 0142')
  await page.getByRole('button', { name: en.signIn.sendCode, exact: true }).click()
  // The notice itself, not the live region that repeats it for screen readers.
  await expect(page.locator('p.notice', { hasText: en.signIn.emailOnly })).toBeVisible()
  await expect(email).toHaveAttribute('aria-invalid', 'true')
  await expect(page.getByText(en.errors.SERVICE_UNAVAILABLE)).toHaveCount(0)
  expect(codesAsked).toEqual([])

  // An email address signs in as ever.
  await page.getByLabel(en.signIn.emailLabel, { exact: true }).fill(ben.email)
  await expect(page.locator('p.notice', { hasText: en.signIn.emailOnly })).toHaveCount(0)
  await signIn(ben, en, en.signIn.emailLabel)
  await expect(page.getByRole('heading', { name: en.profile.firstTitle })).toBeVisible()
})
