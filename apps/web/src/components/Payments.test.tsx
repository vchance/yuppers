// @vitest-environment jsdom
import { afterEach, expect, test } from 'vitest'

import { ACTIVE, DISPUTED, ana } from '../test/fake-service'
import { button, field, press, settle, start, stop, type, until, violations } from '../test/harness'

/*
 * Payment options (`payments.ts`): saving them on the account screen,
 * showing them on a yup, and the payer's sheet, with links that open each
 * app and nothing recorded until the payer says "I've paid" themselves.
 */

afterEach(stop)

const ben = { ...ana, id: 'b0000000-0000-4000-8000-000000000002', display_name: 'Ben Ortiz' }

const DANA = {
  venmo: 'dana-fixes',
  cash_app: 'DanaFixes',
  paypal: 'DanaFixes',
  zelle: '+12025550142',
}

const lastSent = (service: { sent: { call: string; body: unknown }[] }, call: string) =>
  [...service.sent].reverse().find((sent) => sent.call === call)?.body

function sectionHeaded(text: string): HTMLElement | null {
  const found = [...document.querySelectorAll('h2')].find((h) => h.textContent === text)
  return found?.closest('section') ?? null
}

test('the account saves payment options, says which entries are wrong, and removes them', async () => {
  const { service, wording } = await start('/account', ana)
  const w = wording.payments
  await until(() => sectionHeaded(w.heading) !== null, 'the payment options section')
  await until(() => !(field(w.venmoLabel) as HTMLInputElement).disabled, 'the form to load')
  expect(await violations()).toEqual([])

  // Wrong entries are named, and the keyboard goes to the first; nothing is sent.
  await type(field(w.venmoLabel), 'ana')
  await type(field(w.paypalLabel), 'ana_pays')
  await press(button(w.save))
  await settle()
  const section = sectionHeaded(w.heading)!
  expect(section.textContent).toContain(w.venmoInvalid)
  expect(section.textContent).toContain(w.paypalInvalid)
  expect(field(w.venmoLabel).getAttribute('aria-invalid')).toBe('true')
  expect(document.activeElement).toBe(field(w.venmoLabel))
  expect(lastSent(service, 'PUT /v1/me/payment-handles')).toBeUndefined()

  await type(field(w.venmoLabel), '@ana-pays')
  await type(field(w.paypalLabel), 'paypal.me/AnaPays')
  await type(field(w.zelleLabel), '(202) 555-0142')
  await press(button(w.save))
  await until(() => section.textContent!.includes(w.saved), 'saved')
  expect(lastSent(service, 'PUT /v1/me/payment-handles')).toEqual({
    venmo: 'ana-pays',
    cash_app: null,
    paypal: 'AnaPays',
    zelle: '+12025550142',
  })
  expect((field(w.zelleLabel) as HTMLInputElement).value).toBe('(202) 555-0142')

  await press(button(w.remove))
  await until(() => section.textContent!.includes(w.removed), 'removed')
  expect(service.sent.some((sent) => sent.call === 'DELETE /v1/me/payment-handles')).toBe(true)
  expect((field(w.venmoLabel) as HTMLInputElement).value).toBe('')
})

test('a payee shows their options on a yup with a box that is off until ticked', async () => {
  const { service, wording } = await start(`/exchanges/${ACTIVE}`, ana, 'en', (fake) => {
    fake.handles = { ...DANA }
  })
  const w = wording.payments
  await until(() => sectionHeaded(w.showHeading) !== null, 'the payment options box')
  const box = field(w.showLabel) as HTMLInputElement
  expect(box.type).toBe('checkbox')
  expect(box.checked).toBe(false)
  expect(document.getElementById(box.getAttribute('aria-describedby')!)?.textContent).toBe(
    w.showHint.replace('{name}', 'Ben Ortiz'),
  )
  await press(box)
  await until(() => service.shown.has(ACTIVE), 'shown')
  expect(lastSent(service, `PUT /v1/exchanges/${ACTIVE}/payment-options`)).toEqual({ on: true })
  await until(() => (field(w.showLabel) as HTMLInputElement).checked, 'ticked')
  await press(field(w.showLabel))
  await until(() => !service.shown.has(ACTIVE), 'hidden again')
  expect(lastSent(service, `PUT /v1/exchanges/${ACTIVE}/payment-options`)).toEqual({ on: false })
  expect(await violations()).toEqual([])
})

test('a payee with nothing saved is pointed to the account instead', async () => {
  const { wording } = await start(`/exchanges/${ACTIVE}`, ana)
  const w = wording.payments
  await until(() => sectionHeaded(w.showHeading) !== null, 'the payment options box')
  const link = [...document.querySelectorAll('a')].find((a) => a.textContent === w.addInAccount)
  expect(link?.getAttribute('href')).toBe('/account')
})

test('the payer sees Pay, and a dialog of text-only links with the amount and note', async () => {
  const { service, wording } = await start(`/exchanges/${DISPUTED}`, ben, 'en', (fake) => {
    fake.theirs = { ...DANA }
  })
  const w = wording.payments
  const pay = 'Pay Ana Ruiz $450.00'
  await until(() => [...document.querySelectorAll('button')].some((b) => b.textContent === pay), 'Pay')
  const payButton = button(pay)
  expect(payButton.getAttribute('aria-haspopup')).toBe('dialog')
  await press(payButton)
  await until(() => document.querySelector('dialog.sheet') !== null, 'the sheet')
  const sheet = document.querySelector('dialog.sheet') as HTMLDialogElement
  await until(() => sheet.querySelectorAll('a.pay-app').length === 3, 'the links')

  // A proper dialog: named by its title, described by who added the options.
  expect(sheet.hasAttribute('open')).toBe(true)
  const title = document.getElementById(sheet.getAttribute('aria-labelledby')!)
  expect(title?.textContent).toBe(pay)
  expect(document.activeElement).toBe(title)
  expect(document.getElementById(sheet.getAttribute('aria-describedby')!)?.textContent).toBe(
    'Added by Ana Ruiz. Yuppers doesn’t check it or move money. Make sure it’s the right person.',
  )
  expect(sheet.textContent).toContain(wording.terms.moneyOutside)
  expect(sheet.textContent).toContain(w.appTerms)
  // The exchange was read again as it opened.
  expect(service.sent.filter((sent) => sent.call === `GET /v1/exchanges/${DISPUTED}`).length).toBeGreaterThan(1)

  // Text-only links that open each app in a new tab, prefilled.
  const links = [...sheet.querySelectorAll<HTMLAnchorElement>('a.pay-app')]
  expect(sheet.querySelectorAll('img, svg').length).toBe(0)
  expect(links.map((link) => link.textContent)).toEqual([
    `${w.open.venmo} ${wording.help.newTab}`,
    `${w.open.cash_app} ${wording.help.newTab}`,
    `${w.open.paypal} ${wording.help.newTab}`,
  ])
  expect(links.map((link) => link.href)).toEqual([
    'https://venmo.com/u/dana-fixes?txn=pay&amount=450&note=Payment%20for%20the%20repair%20%C2%B7%20yup%20DSPT-8M3R',
    'https://cash.app/$DanaFixes/450',
    'https://www.paypal.me/DanaFixes/450USD',
  ])
  for (const link of links) {
    expect(link.target).toBe('_blank')
    expect(link.rel).toBe('noopener noreferrer')
  }
  expect(sheet.textContent).toContain('Venmo @dana-fixes')
  expect(sheet.textContent).toContain('Cash App $DanaFixes')

  // Zelle: no link, the number to copy.
  expect(sheet.textContent).toContain(w.zelleNoLinks)
  expect(sheet.textContent).toContain('(202) 555-0142')
  const copies = [...sheet.querySelectorAll('button.copy')].map((b) => b.getAttribute('aria-label'))
  expect(copies).toEqual(['Copy Amount', 'Copy Note', 'Copy Zelle'])

  expect(await violations()).toEqual([])

  // Opening a link records nothing; "I've paid" opens the usual claim, which
  // the payer still sends.
  const before = service.sent.length
  links[0].addEventListener('click', (event) => event.preventDefault())
  await press(links[0])
  expect(service.sent.slice(before).some((sent) => sent.call.startsWith('POST'))).toBe(false)
  await press(button(wording.exchange.moneyMoves.CLAIM))
  await until(() => document.querySelector('dialog.sheet') === null, 'the sheet to close')
  expect(service.sent.some((sent) => sent.call === `POST /v1/exchanges/${DISPUTED}/commands`)).toBe(false)
  const panel = document.querySelector('.panel')
  expect(panel?.textContent).toContain(wording.exchange.moneyMoveText.CLAIM.replace(/\{name\}/g, 'Ana Ruiz'))
})

test('with no payment options shown, there is no Pay and the claim works as before', async () => {
  const { wording } = await start(`/exchanges/${DISPUTED}`, ben)
  await until(
    () => [...document.querySelectorAll('button')].some((b) => b.textContent === wording.exchange.moneyMoves.CLAIM),
    "I've paid",
  )
  expect([...document.querySelectorAll('button')].some((b) => b.textContent?.startsWith('Pay '))).toBe(false)
})

test('options the payee stopped showing are not offered once the sheet opens', async () => {
  const { service, wording } = await start(`/exchanges/${DISPUTED}`, ben, 'en', (fake) => {
    fake.theirs = { ...DANA }
  })
  const pay = 'Pay Ana Ruiz $450.00'
  await until(() => [...document.querySelectorAll('button')].some((b) => b.textContent === pay), 'Pay')
  service.theirs = null
  await press(button(pay))
  const gone = wording.payments.gone.replace('{name}', 'Ana Ruiz')
  await until(() => document.querySelector('dialog.sheet')?.textContent?.includes(gone) ?? false, 'gone')
  expect(document.querySelectorAll('a.pay-app').length).toBe(0)
  await press(button(wording.payments.close))
  await until(
    () => ![...document.querySelectorAll('button')].some((b) => b.textContent === pay),
    'Pay to go',
  )
})
