import { ApiPerson } from './support/api'
import { expect, test } from './support/fixtures'
import { acceptOpen, button, join, signUp, title } from './support/flows'
import { en, fill } from './support/wording'

/*
 * Payment options in the app (README, "Payment options"): the payee saves
 * them on the account screen and shows them on one yup; the payer sees
 * "Pay", a sheet whose buttons open each app by its web address, prefilled,
 * and Zelle to copy, and still records the payment only by saying "I've
 * paid" themselves. Not shown, nothing of it appears.
 */

const w = en.payments
const DEPOSIT = 'Fence deposit'
const PAY = fill(w.payButton, { name: 'Ana', amount: '$100.50' })

test('the payee shows payment options; the payer opens the app and says they paid', async ({
  person,
}) => {
  const ana = await person('Ana')
  await signUp(ana)
  await ana.page.getByRole('link', { name: en.nav.account }).click()
  await expect(title(ana.page, en.nav.account)).toBeVisible()
  await expect(ana.page.getByLabel(w.venmoLabel, { exact: true })).toBeEditable()
  await ana.page.getByLabel(w.venmoLabel, { exact: true }).fill('@ana-fixes')
  await ana.page.getByLabel(w.cashAppLabel, { exact: true }).fill('$AnaFixes')
  await ana.page.getByLabel(w.paypalLabel, { exact: true }).fill('AnaFixes')
  await ana.page.getByLabel(w.zelleLabel, { exact: true }).fill('ana@zelle.example')
  await button(ana.page, w.save).click()
  await expect(ana.page.getByText(w.saved)).toBeVisible()

  // Ana proposes the fence job from a second session of hers; Ben signs in the app.
  const anaApi = await ApiPerson.signUp('Ana', ana.email)
  const { id, link } = await anaApi.propose('Ben', [
    { from: 'A', kind: 'SERVICE', description: 'Fence repair' },
    { from: 'B', kind: 'MONEY', description: DEPOSIT, amountMinor: 10050 },
  ])
  const ben = await person('Ben')
  await join(ben, link)
  await acceptOpen(ben.page)
  await anaApi.confirmCounterparty(id)

  // Nothing to pay with until Ana shows them.
  await ben.page.reload()
  await expect(button(ben.page, en.exchange.moneyMoves.CLAIM)).toBeVisible()
  await expect(button(ben.page, PAY)).toHaveCount(0)

  await ana.page.goto(`/exchanges/${id}`)
  const toggle = ana.page.getByLabel(w.showLabel, { exact: true })
  await expect(toggle).not.toBeChecked()
  await toggle.check()
  await expect(ana.page.getByText(fill(w.shownNow, { name: 'Ben' }))).toBeVisible()

  await ben.page.reload()
  await button(ben.page, PAY).click()
  const sheet = ben.page.getByTestId('pay-sheet')
  await expect(sheet).toBeVisible()
  await expect(sheet.getByText(fill(w.sheetTitle, { name: 'Ana', amount: '$100.50' }))).toBeVisible()
  await expect(sheet.getByText(fill(w.addedBy, { name: 'Ana' }))).toBeVisible()
  await expect(sheet.getByText('Venmo @ana-fixes')).toBeVisible()
  await expect(sheet.getByText(w.zelleNoLinks)).toBeVisible()
  await expect(sheet.getByText('ana@zelle.example')).toBeVisible()

  // Each button opens its app's web address, which the app claims; the
  // test answers for the apps' sites. Nothing is recorded by it.
  await ben.context.route(/^https:\/\/(venmo\.com|cash\.app|www\.paypal\.me)\//, (route) =>
    route.fulfill({ status: 200, contentType: 'text/plain', body: 'payment app' }),
  )
  for (const [label, url] of [
    [
      w.open.venmo,
      /^https:\/\/venmo\.com\/u\/ana-fixes\?txn=pay&amount=100\.50&note=Fence%20deposit%20%C2%B7%20yup%20[A-Z0-9]{4}-[A-Z0-9]{4}$/,
    ],
    [w.open.cash_app, /^https:\/\/cash\.app\/\$AnaFixes\/100\.50$/],
    [w.open.paypal, /^https:\/\/www\.paypal\.me\/AnaFixes\/100\.50USD$/],
  ] as const) {
    const opened = ben.context.waitForEvent('page')
    await button(sheet, label).click()
    const tab = await opened
    expect(tab.url()).toMatch(url)
    await tab.close()
  }
  const seen = (await anaApi.view(id)) as unknown as { contributions: { status: string }[] }
  expect(seen.contributions.map((item) => item.status)).toEqual(['PENDING', 'PENDING'])

  // "I've paid" opens the usual claim, which Ben sends himself.
  await ben.page.getByTestId('pay-sheet-paid').click()
  await expect(sheet).toBeHidden()
  const panel = ben.page.getByRole('heading', { name: en.exchange.moneyMoves.CLAIM, level: 3 })
  await expect(panel).toBeVisible()
  await button(ben.page, en.exchange.moneyMoves.CLAIM).last().click()
  await expect(panel).toBeHidden()
  await expect(ben.page.getByText(en.moneyStatus.CLAIMED)).toBeVisible()
  await expect(button(ben.page, PAY)).toHaveCount(0)
})

test('with payment options not shown, the payer pays as before', async ({ person, email }) => {
  const anaApi = await ApiPerson.signUp('Ana', email('ana'))
  await anaApi.call('PUT', '/v1/me/payment-handles', { venmo: 'ana-fixes' })
  const { id, link, contributionIds } = await anaApi.propose('Ben', [
    { from: 'A', kind: 'SERVICE', description: 'Fence repair' },
    { from: 'B', kind: 'MONEY', description: DEPOSIT, amountMinor: 10050 },
  ])
  const ben = await person('Ben')
  await join(ben, link)
  await acceptOpen(ben.page)
  await anaApi.confirmCounterparty(id)
  // Shown and then turned off again.
  await anaApi.call('PUT', `/v1/exchanges/${id}/payment-options`, { on: true })
  await anaApi.call('PUT', `/v1/exchanges/${id}/payment-options`, { on: false })

  await ben.page.reload()
  await expect(button(ben.page, en.exchange.moneyMoves.CLAIM)).toBeVisible()
  await expect(button(ben.page, PAY)).toHaveCount(0)
  await button(ben.page, en.exchange.moneyMoves.CLAIM).click()
  await button(ben.page, en.exchange.moneyMoves.CLAIM).last().click()
  await expect(ben.page.getByText(en.moneyStatus.CLAIMED)).toBeVisible()
  await anaApi.contribution(id, contributionIds[1], 'CONFIRM')
  await ben.page.reload()
  await expect(ben.page.getByText(en.moneyStatus.ACCEPTED)).toBeVisible()
})
