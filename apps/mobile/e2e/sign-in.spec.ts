import { codeFrom } from './support/codes'
import { expect, test } from './support/fixtures'
import { button, signIn, title } from './support/flows'
import { en, fill } from './support/wording'

/*
 * Signing in asks for what the service can send codes to (README, "Signing
 * in"). The API here writes codes to its log, phone codes included, so it
 * offers both; a deployment without text messages is stood in for by
 * answering `GET /v1/meta` in the browser as such a service would.
 */

test('the service offers phone numbers here, and the screen asks for US numbers', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana

  await page.goto('/')
  await expect(title(page, en.signIn.title)).toBeVisible()
  await expect(page.getByLabel(en.signIn.identifierLabel)).toBeVisible()
  // US numbers only, written as they are there: no country code asked for.
  await expect(page.getByText(en.signIn.identifierHintUs)).toBeVisible()
  await signIn(ana)
  await expect(page.getByText(en.profile.firstIntro)).toBeVisible()
})

test('a phone number gets its code by text only once the box beside it is ticked', async ({ person }) => {
  const sam = await person('Sam')
  const { page } = sam
  // A US area code: +1 covers Canada and the Caribbean too, which the
  // service does not text.
  const digits = () => Math.floor(Math.random() * 10)
  const area = ['212', '415', '617', '713', '917'][Math.floor(Math.random() * 5)]
  const phone = `+1${area}${2 + (digits() % 8)}${Array.from({ length: 6 }, digits).join('')}`
  // As people in the US write it, and as the screens show it, without +1.
  const american = `(${phone.slice(2, 5)}) ${phone.slice(5, 8)}-${phone.slice(8)}`
  const asked: unknown[] = []
  page.on('request', (request) => {
    if (request.url().endsWith('/v1/auth/codes')) asked.push(request.postDataJSON())
  })

  await page.goto('/')
  await expect(title(page, en.signIn.title)).toBeVisible()
  const identifier = page.getByLabel(en.signIn.identifierLabel)
  const box = page.getByRole('checkbox', { name: en.smsCode.signIn, exact: true })
  const send = button(page, en.signIn.sendCode)
  // An email address: no box.
  await identifier.fill(sam.email)
  await expect(box).toHaveCount(0)

  // A number, typed without +1: the box, unticked, and the button waiting
  // for it, saying why.
  await identifier.fill(american)
  await expect(box).toBeVisible()
  await expect(box).not.toBeChecked()
  await expect(send).toBeDisabled()
  await expect(page.getByText(en.smsCode.tickToSend, { exact: true })).toBeVisible()
  await expect(page.getByText('Message and data rates may apply. Reply STOP to opt out.')).toHaveCount(0)
  await send.click({ force: true })
  expect(asked).toEqual([])

  await box.click()
  await expect(box).toBeChecked()
  await expect(send).toBeEnabled()
  // The log masks the number but for its last two digits.
  const code = await codeFrom(`+1••••••••${phone.slice(-2)}`, 'sign-in', () => send.click())
  // Sent in E.164, and shown the American way.
  await expect(page.getByText(fill(en.signIn.codeSent, { identifier: american }))).toBeVisible()
  expect(asked).toEqual([{ identifier: phone, sms_consent: { version: expect.any(String), language: 'en' } }])
  await page.getByLabel(en.signIn.codeLabel).fill(code)
  await button(page, en.signIn.submit).click()
  await expect(page.getByText(en.profile.firstIntro)).toBeVisible()
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
  await expect(email).toBeVisible()
  await expect(page.getByLabel(en.signIn.identifierLabel)).toHaveCount(0)

  await email.fill('+1 202 555 0142')
  await button(page, en.signIn.sendCode).click()
  await expect(page.getByText(en.signIn.emailOnly)).toBeVisible()
  await expect(page.getByText(en.errors.SERVICE_UNAVAILABLE)).toHaveCount(0)
  expect(codesAsked).toEqual([])

  // An email address signs in as ever.
  await signIn(ben, en, en.signIn.emailLabel)
  await expect(page.getByText(en.profile.firstIntro)).toBeVisible()
})
