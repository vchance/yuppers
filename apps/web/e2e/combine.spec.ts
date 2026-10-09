import { apiLog } from './support/env'
import { codeFrom } from './support/codes'
import { expect, test } from './support/fixtures'
import { propose, signUp, stateTag } from './support/flows'
import { american, codesTo, number, waitFor } from './support/texts'
import { en, fill } from './support/wording'

/*
 * The account's email address and phone number, and combining two accounts
 * (README, "Combining accounts"), through the screens.
 */

/** Signs in with a phone number from the sign-in form on the page, ticking the box. */
async function signInByPhone(page: import('@playwright/test').Page, phone: string) {
  await page.getByLabel(en.signIn.identifierLabel, { exact: true }).fill(american(phone))
  await page.getByRole('checkbox', { name: en.smsCode.signIn, exact: true }).check()
  const before = codesTo(phone, apiLog).length
  await page.getByRole('button', { name: en.signIn.sendCode, exact: true }).click()
  const code = await waitFor(() => codesTo(phone, apiLog)[before], 'the sign-in code')
  await page.getByLabel(en.signIn.codeLabel).fill(code)
  await page.getByRole('button', { name: en.signIn.submit, exact: true }).click()
}

test('signed in by phone, an invitation sent to the email of another account combines the two and opens', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  // Bruno already has an account by email.
  await signUp(bruno)
  await signUp(ana)
  const { id, link } = await propose(
    ana,
    bruno,
    [{ from: 'me', kind: 'ITEM', description: 'A set of garden chairs' }],
    { invitee: bruno.email },
  )

  // On his phone he signs in by number, a new account, and opens the link.
  const phone = await person('Bruno')
  const number_ = number()
  await phone.page.goto(link)
  await signInByPhone(phone.page, number_)
  // Only the kind is said; he types the address.
  await expect(phone.page.getByRole('heading', { name: en.invitation.sentToEmail })).toBeVisible()
  await expect(phone.page.getByText(bruno.email)).toHaveCount(0)
  const typed = phone.page.getByLabel(en.identifiers.newEmailLabel, { exact: true })
  const send = phone.page.getByRole('button', { name: en.identifiers.sendCode, exact: true })
  // Another address is refused, saying nothing more.
  await typed.fill(`someone-else-${Date.now()}@example.test`)
  await send.click()
  await expect(phone.page.getByText(en.errors.NOT_INVITED_ADDRESS)).toBeVisible()
  await typed.fill(bruno.email)
  const code = await codeFrom(bruno.email, 'sign-in', () => send.click())
  await phone.page.getByLabel(en.signIn.codeLabel).fill(code)
  await phone.page.getByRole('button', { name: en.invitation.addAndOpen, exact: true }).click()

  // The address is his other account's: the offer to combine, saying what moves.
  const offer = phone.page.getByRole('group', { name: en.combine.headingEmail })
  await expect(offer).toBeVisible()
  await expect(offer.getByText(fill(en.combine.otherName, { name: 'Bruno' }))).toBeVisible()
  await expect(offer.getByText(en.combine.cannotUndo)).toBeVisible()
  await offer.getByRole('button', { name: en.combine.confirm, exact: true }).click()

  // Combined: he lands in the invitation, named, as the one it was for.
  await phone.page.waitForURL(`**/exchanges/${id}`)
  await expect(stateTag(phone.page)).toHaveText(en.states.NEGOTIATING)
  // His account has both now.
  await phone.page.goto('/account')
  await expect(phone.page.getByText(bruno.email, { exact: true })).toBeVisible()
  await expect(phone.page.getByText(american(number_), { exact: true })).toBeVisible()
  // The email account's own browser is signed out.
  await bruno.page.reload()
  await expect(bruno.page.getByRole('heading', { name: en.signIn.title, level: 1 })).toBeVisible()
})

test('the email is changed with a code to it first, and the phone removed with a code to the email', async ({
  person,
}) => {
  const cleo = await person('Cleo')
  await signUp(cleo)
  const { page } = cleo
  await page.goto('/account')
  const w = en.identifiers
  await expect(page.getByRole('heading', { name: w.heading })).toBeVisible()
  // Only the email: it cannot be removed, and the page says why.
  const removes = page.getByRole('button', { name: w.remove, exact: true })
  await expect(removes).toHaveCount(1)
  await expect(removes).toBeDisabled()
  await expect(removes).toHaveAccessibleDescription(w.onlyEmail)

  // Add a phone number, by text once the box is ticked.
  const phone = number()
  await page.getByRole('button', { name: w.add, exact: true }).click()
  const adding = page.getByRole('group', { name: w.addPhoneTitle })
  await adding.getByLabel(w.newPhoneLabel, { exact: true }).fill(american(phone))
  await adding.getByRole('checkbox', { name: en.smsCode.verifyNumber, exact: true }).check()
  const before = codesTo(phone, apiLog).length
  await adding.getByRole('button', { name: w.sendCode, exact: true }).click()
  const code = await waitFor(() => codesTo(phone, apiLog)[before], 'the code for the number')
  await adding.getByLabel(en.signIn.codeLabel).fill(code)
  await adding.getByRole('button', { name: w.confirm, exact: true }).click()
  await expect(page.getByText(american(phone), { exact: true })).toBeVisible()

  // Change the email: first a code to the one it replaces, then the new one's.
  const email = `cleo-new-${Date.now()}@example.test`
  await page.getByRole('button', { name: w.change, exact: true }).first().click()
  const changing = page.getByRole('group', { name: w.changeEmailTitle })
  await expect(changing.getByText(fill(w.proveIntro, { identifier: cleo.email }))).toBeVisible()
  await expect(
    changing.getByRole('button', { name: fill(w.proveOther, { identifier: american(phone) }) }),
  ).toBeVisible()
  const proving = await codeFrom(cleo.email, 'sign-in', () =>
    changing.getByRole('button', { name: fill(w.proveSend, { identifier: cleo.email }) }).click(),
  )
  await changing.getByLabel(en.signIn.codeLabel).fill(proving)
  await changing.getByRole('button', { name: w.proveConfirm, exact: true }).click()
  await changing.getByLabel(w.newEmailLabel, { exact: true }).fill(email)
  const newCode = await codeFrom(email, 'sign-in', () =>
    changing.getByRole('button', { name: w.sendCode, exact: true }).click(),
  )
  await changing.getByLabel(en.signIn.codeLabel).fill(newCode)
  await changing.getByRole('button', { name: w.confirm, exact: true }).click()
  await expect(page.getByText(email, { exact: true })).toBeVisible()

  // Remove the phone: the code goes to the email, which stays.
  await expect(removes).toHaveCount(2)
  await removes.nth(1).click()
  const panel = page.getByRole('group', { name: w.removePhoneTitle })
  await expect(panel.getByText(fill(w.removeIntro, { staying: email }))).toBeVisible()
  const removal = await codeFrom(email, 'sign-in', () =>
    panel.getByRole('button', { name: fill(w.sendRemovalCode, { staying: email }) }).click(),
  )
  await panel.getByLabel(en.signIn.codeLabel).fill(removal)
  await panel.getByRole('button', { name: w.removeConfirm, exact: true }).click()
  await expect(page.getByText(american(phone), { exact: true })).toHaveCount(0)
  await expect(page.getByText(w.removed)).toBeVisible()
  await expect(removes).toHaveCount(1)
  await expect(removes).toBeDisabled()
})
