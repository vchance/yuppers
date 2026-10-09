import type { Locator, Page } from '@playwright/test'

import { ApiPerson } from './support/api'
import { codeFrom } from './support/codes'
import { expect, test } from './support/fixtures'
import { button, signUp, title } from './support/flows'
import { en, fill } from './support/wording'

/*
 * The account's email address and phone number, and combining two accounts
 * (README, "Combining accounts"), on the app's screens.
 */

/** A US number nobody else uses, and how the screens write it. */
function usNumber(): { phone: string; american: string } {
  const digits = () => Math.floor(Math.random() * 10)
  const area = ['212', '415', '617', '713', '917'][Math.floor(Math.random() * 5)]
  const phone = `+1${area}${2 + (digits() % 8)}${Array.from({ length: 6 }, digits).join('')}`
  return { phone, american: `(${phone.slice(2, 5)}) ${phone.slice(5, 8)}-${phone.slice(8)}` }
}

/** The log masks a number but for its last two digits. */
const inLog = (phone: string) => `+1••••••••${phone.slice(-2)}`

const shown = (locator: Locator) => locator.filter({ visible: true })

async function signInByPhone(page: Page, phone: string, american: string) {
  await shown(page.getByLabel(en.signIn.identifierLabel)).fill(american)
  await shown(page.getByRole('checkbox', { name: en.smsCode.signIn, exact: true })).click()
  const code = await codeFrom(inLog(phone), 'sign-in', () =>
    shown(button(page, en.signIn.sendCode)).click(),
  )
  await shown(page.getByLabel(en.signIn.codeLabel)).fill(code)
  await shown(button(page, en.signIn.submit)).click()
}

test('signed in by phone, an invitation sent to the email of another account combines the two and opens', async ({
  person,
  email,
}) => {
  const ana = await ApiPerson.signUp('Ana', email('ana'))
  const brunoEmail = email('bruno')
  await ApiPerson.signUp('Bruno', brunoEmail)
  const { id, link } = await ana.propose(
    'Bruno',
    [{ from: 'A', kind: 'ITEM', description: 'A set of garden chairs' }],
    { boundTo: brunoEmail },
  )

  const bruno = await person('Bruno')
  const { page } = bruno
  const { phone, american } = usNumber()
  await page.goto(link)
  await expect(title(page, en.invitation.signedOutTitle)).toBeVisible()
  await signInByPhone(page, phone, american)

  const masked = `${brunoEmail[0]}•••@${brunoEmail.split('@')[1]}`
  await expect(
    page.getByRole('heading', { name: fill(en.invitation.sentTo, { identifier: masked }) }),
  ).toBeVisible()
  await expect(page.getByText(brunoEmail)).toHaveCount(0)
  const code = await codeFrom(brunoEmail, 'sign-in', () =>
    button(page, fill(en.invitation.sendAddressCode, { identifier: masked })).click(),
  )
  await shown(page.getByLabel(en.signIn.codeLabel)).fill(code)
  await button(page, en.invitation.addAndOpen).click()

  await expect(page.getByRole('heading', { name: en.combine.headingEmail })).toBeVisible()
  await expect(page.getByText(`• ${fill(en.combine.otherName, { name: 'Bruno' })}`)).toBeVisible()
  await expect(page.getByText(en.combine.cannotUndo, { exact: false })).toBeVisible()
  await button(page, en.combine.confirm).click()

  await page.waitForURL(`**/exchanges/${id}`)
})

test('the phone is removed from an account that has both, with a code to the email', async ({
  person,
}) => {
  const cleo = await person('Cleo')
  await signUp(cleo)
  const { page } = cleo
  await page.getByRole('link', { name: en.nav.account }).click()
  await expect(title(page, en.nav.account)).toBeVisible()
  const w = en.identifiers
  await expect(page.getByRole('heading', { name: w.heading })).toBeVisible()
  const removeEmail = button(page, `${w.remove}: ${en.profile.emailLabel}`)
  await expect(removeEmail).toBeDisabled()
  await expect(page.getByText(w.onlyEmail).first()).toBeVisible()

  const { phone, american } = usNumber()
  await button(page, `${w.add}: ${en.profile.phoneLabel}`).click()
  await page.getByRole('textbox', { name: w.newPhoneLabel, exact: true }).fill(american)
  await page.getByRole('checkbox', { name: en.smsCode.verifyNumber, exact: true }).click()
  const code = await codeFrom(inLog(phone), 'sign-in', () => button(page, w.sendCode).click())
  await page.getByLabel(en.signIn.codeLabel).fill(code)
  await button(page, w.confirm).click()
  await expect(page.getByText(american, { exact: true })).toBeVisible()

  await button(page, `${w.remove}: ${en.profile.phoneLabel}`).click()
  await expect(page.getByText(fill(w.removeIntro, { staying: cleo.email }))).toBeVisible()
  const removal = await codeFrom(cleo.email, 'sign-in', () =>
    button(page, fill(w.sendRemovalCode, { staying: cleo.email })).click(),
  )
  await page.getByLabel(en.signIn.codeLabel).fill(removal)
  await button(page, w.removeConfirm).click()
  await expect(page.getByText(w.removed)).toBeVisible()
  await expect(page.getByText(american, { exact: true })).toHaveCount(0)
  await expect(removeEmail).toBeDisabled()
})
