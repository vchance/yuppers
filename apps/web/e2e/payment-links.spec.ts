import type { Page } from '@playwright/test'

import { expect, test, type Person } from './support/fixtures'
import {
  acceptOpen,
  addItems,
  agree,
  agreedItem,
  confirmClaimant,
  inviteFor,
  join,
  move,
  signUp,
  startExchange,
  type ItemSpec,
} from './support/flows'
import { en, fill } from './support/wording'

/*
 * Payment options (README, "Payment options"): the payee saves them on the
 * account and shows them on one yup; the payer sees "Pay", a sheet of
 * text-only links that open each app prefilled, and Zelle to copy, and still
 * records the payment only by saying "I've paid" themselves. With them not
 * shown, nothing of it appears and the claim works as before.
 */

const w = en.payments
const FENCE = 'Fence repair'
const DEPOSIT = 'Fence deposit & gate #2'
const ITEMS: ItemSpec[] = [
  { from: 'me', kind: 'SERVICE', description: FENCE },
  { from: 'them', kind: 'MONEY', description: DEPOSIT, amount: '100.50' },
]

/** Saves payment options on the account screen. */
async function saveOptions(page: Page): Promise<void> {
  await page.goto('/account')
  const section = page.getByRole('region', { name: w.heading, exact: true })
  await expect(section.getByLabel(w.venmoLabel)).toBeEnabled()
  await section.getByLabel(w.venmoLabel).fill('@ana-fixes')
  await section.getByLabel(w.cashAppLabel).fill('$AnaFixes')
  await section.getByLabel(w.paypalLabel).fill('paypal.me/AnaFixes')
  await section.getByLabel(w.zelleLabel).fill('202-555-0142')
  await section.getByRole('button', { name: w.save, exact: true }).click()
  await expect(section.getByText(w.saved)).toBeVisible()
  await page.reload()
  await expect(page.getByLabel(w.venmoLabel)).toHaveValue('ana-fixes')
  await expect(page.getByLabel(w.zelleLabel)).toHaveValue('(202) 555-0142')
}

/** Ana proposes the fence job, turning on her payment options as she sends it; Bruno signs. */
async function agreeShowingOptions(ana: Person, bruno: Person): Promise<void> {
  const { page } = ana
  await startExchange(ana)
  await page.getByLabel(en.composer.otherName).fill(bruno.name)
  await inviteFor(page, null)
  await addItems(page, ITEMS)
  await page.getByRole('button', { name: en.composer.review, exact: true }).click()
  const alsoShow = page.getByLabel(w.showWhenSigning)
  await expect(alsoShow).not.toBeChecked()
  await alsoShow.check()
  await page.getByLabel(en.consent.agree).check()
  await page.getByRole('button', { name: en.composer.signAndSend, exact: true }).click()
  const field = page.getByLabel(en.invitationLink.linkLabel, { exact: true })
  await expect(field).toHaveValue(/\/en\/i#/)
  const link = await field.inputValue()
  await join(bruno, link)
  await acceptOpen(bruno)
  await confirmClaimant(ana)
}

function payButton(page: Page) {
  return agreedItem(page, DEPOSIT).getByRole('button', {
    name: fill(w.payButton, { name: 'Ana', amount: '$100.50' }),
    exact: true,
  })
}

test('the payee shows payment options; the payer pays in the app and says so themselves', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  await signUp(ana)
  await saveOptions(ana.page)
  await agreeShowingOptions(ana, bruno)

  // Ana's yup says her options are shown, and can stop showing them.
  const shown = ana.page.getByRole('region', { name: w.showHeading, exact: true })
  await expect(shown.getByLabel(w.showLabel)).toBeChecked()

  // Bruno owes the deposit: "Pay", and a dialog.
  await bruno.page.reload()
  await payButton(bruno.page).click()
  const sheet = bruno.page.getByRole('dialog', { name: fill(w.sheetTitle, { name: 'Ana', amount: '$100.50' }) })
  await expect(sheet).toBeVisible()
  await expect(sheet.getByText(fill(w.addedBy, { name: 'Ana' }))).toBeVisible()
  await expect(sheet.getByText(w.appTerms, { exact: false })).toBeVisible()
  // Saved before the agreement came into force: nothing to warn about.
  await expect(sheet.locator('.pay-changed')).toHaveCount(0)

  // Text-only links, each opening its app in a new tab, prefilled.
  const venmo = sheet.getByRole('link', { name: new RegExp(`^${w.open.venmo}`) })
  const cashApp = sheet.getByRole('link', { name: new RegExp(`^${w.open.cash_app}`) })
  const paypal = sheet.getByRole('link', { name: new RegExp(`^${w.open.paypal}`) })
  await expect(venmo).toHaveAttribute(
    'href',
    /^https:\/\/venmo\.com\/u\/ana-fixes\?txn=pay&amount=100\.50&note=Fence%20deposit%20%26%20gate%20%232%20%C2%B7%20yup%20[A-Z0-9]{4}-[A-Z0-9]{4}$/,
  )
  await expect(cashApp).toHaveAttribute('href', 'https://cash.app/$AnaFixes/100.50')
  await expect(paypal).toHaveAttribute('href', 'https://www.paypal.me/AnaFixes/100.50USD')
  for (const link of [venmo, cashApp, paypal]) {
    await expect(link).toHaveAttribute('target', '_blank')
    await expect(link).toHaveAttribute('rel', 'noopener noreferrer')
  }
  await expect(sheet.locator('img, svg')).toHaveCount(0)

  // Zelle: no link, the number to copy.
  await expect(sheet.getByText(w.zelleNoLinks)).toBeVisible()
  await expect(sheet.getByText('(202) 555-0142')).toBeVisible()
  await sheet.getByRole('button', { name: fill(w.copyWhat, { what: w.zelleHeading }) }).click()
  expect(await bruno.page.evaluate(() => navigator.clipboard.readText())).toBe('(202) 555-0142')
  await sheet.getByRole('button', { name: fill(w.copyWhat, { what: w.amountLabel }) }).click()
  expect(await bruno.page.evaluate(() => navigator.clipboard.readText())).toBe('100.50')

  // Opening a link records nothing. (The apps' sites are not reached from the test.)
  await bruno.context.route(/^https:\/\/(venmo\.com|cash\.app|www\.paypal\.me)\//, (route) =>
    route.fulfill({ status: 200, contentType: 'text/plain', body: 'payment app' }),
  )
  const opened = bruno.context.waitForEvent('page')
  await venmo.click()
  const tab = await opened
  expect(tab.url()).toMatch(/^https:\/\/venmo\.com\/u\/ana-fixes\?txn=pay&amount=100\.50/)
  await tab.close()
  await expect(agreedItem(bruno.page, DEPOSIT).locator('.status')).toHaveText(en.moneyStatus.PENDING)

  // Back on the sheet, "I've paid" opens the usual claim, which Bruno sends himself.
  await sheet.getByRole('button', { name: en.exchange.moneyMoves.CLAIM, exact: true }).click()
  await expect(sheet).toBeHidden()
  const claim = agreedItem(bruno.page, DEPOSIT).getByRole('group', {
    name: en.exchange.moneyMoves.CLAIM,
    exact: true,
  })
  await expect(claim).toBeVisible()
  await expect(agreedItem(bruno.page, DEPOSIT).locator('.status')).toHaveText(en.moneyStatus.PENDING)
  await claim.getByRole('button', { name: en.exchange.moneyMoves.CLAIM, exact: true }).click()
  await expect(agreedItem(bruno.page, DEPOSIT).locator('.status')).toHaveText(en.moneyStatus.CLAIMED)
  // Once said, there is nothing left to pay.
  await expect(payButton(bruno.page)).toHaveCount(0)

  // Ana confirms receiving it, as always.
  await ana.page.reload()
  await move(ana.page, DEPOSIT, en.exchange.moneyMoves.CONFIRM)
  await expect(agreedItem(ana.page, DEPOSIT).locator('.status')).toHaveText(en.moneyStatus.ACCEPTED)
})

test('turned off, or never turned on, nothing shows and the claim works as before', async ({ person }) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  await agree(ana, bruno, ITEMS)
  await saveOptions(ana.page)

  // Saved but not shown on this yup: no "Pay".
  await bruno.page.reload()
  await expect(agreedItem(bruno.page, DEPOSIT).getByRole('button', { name: en.exchange.moneyMoves.CLAIM, exact: true })).toBeVisible()
  await expect(payButton(bruno.page)).toHaveCount(0)

  // Shown, then turned off again: gone from Bruno's view.
  await ana.page.goto(bruno.page.url())
  const toggle = ana.page.getByRole('region', { name: w.showHeading, exact: true }).getByLabel(w.showLabel)
  await expect(toggle).not.toBeChecked()
  await toggle.check()
  await expect(ana.page.getByText(fill(w.shownNow, { name: 'Bruno' }))).toBeVisible()
  await bruno.page.reload()
  await expect(payButton(bruno.page)).toBeVisible()

  // Ana's Venmo username changes while Bruno owes her: the sheet warns
  // beside it, and never shows the old one. (Her options were all saved
  // after the agreement came into force, so each is warned about.)
  await ana.page.goto('/account')
  await ana.page.getByLabel(w.venmoLabel).fill('someone-else')
  await ana.page.getByRole('button', { name: w.save, exact: true }).click()
  await expect(ana.page.getByText(w.saved)).toBeVisible()
  await bruno.page.reload()
  await payButton(bruno.page).click()
  const sheet = bruno.page.getByRole('dialog')
  await expect(sheet.locator('.pay-changed')).toHaveCount(4)
  await expect(
    sheet.locator('.pay-changed').filter({ hasText: 'Ana changed this Venmo username on' }),
  ).toHaveCount(1)
  await expect(sheet).not.toContainText('ana-fixes')
  await sheet.getByRole('button', { name: w.close, exact: true }).click()
  await ana.page.goto(bruno.page.url())
  await toggle.uncheck()
  await expect(ana.page.getByText(w.hiddenNow)).toBeVisible()
  await bruno.page.reload()
  await expect(payButton(bruno.page)).toHaveCount(0)

  // The claim and the confirmation, exactly as without payment options.
  await move(bruno.page, DEPOSIT, en.exchange.moneyMoves.CLAIM)
  await expect(agreedItem(bruno.page, DEPOSIT).locator('.status')).toHaveText(en.moneyStatus.CLAIMED)
  await ana.page.reload()
  await move(ana.page, DEPOSIT, en.exchange.moneyMoves.CONFIRM)
  await expect(agreedItem(ana.page, DEPOSIT).locator('.status')).toHaveText(en.moneyStatus.ACCEPTED)
})
