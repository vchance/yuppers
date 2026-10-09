import { expect, test } from 'vitest'

import en from '../wording/en.json'
import es from '../wording/es.json'
import {
  addedApps,
  appsToAdd,
  cashAppUrl,
  cashtag,
  handleShown,
  linkAmount,
  normalizeHandle,
  paymentNote,
  paymentOptionsKey,
  paymentOptionsSummary,
  payOffered,
  payOptions,
  paypalName,
  paypalUrl,
  showOffered,
  venmoUrl,
  venmoUsername,
  zelleRecipient,
  zelleShown,
} from './payments'

const NOTE = en.payments.note

test('a Venmo username is 5 to 30 letters, digits, - or _, kept without its @', () => {
  expect(venmoUsername('@Dana-Fixes_1')).toBe('Dana-Fixes_1')
  expect(venmoUsername(' dana5 ')).toBe('dana5')
  for (const bad of ['dana', '@dan', 'a'.repeat(31), 'dana fixes', 'dana.fixes', 'dañafixes', '@@danafixes']) {
    expect(venmoUsername(bad), bad).toBeNull()
  }
})

test('a $Cashtag has a letter and at most 20 characters, kept without its $', () => {
  expect(cashtag('$DanaFixes')).toBe('DanaFixes')
  expect(cashtag('d')).toBe('d')
  for (const bad of ['$', '12345', 'a'.repeat(21), 'dana-fixes', 'dana fixes', '$$dana']) {
    expect(cashtag(bad), bad).toBeNull()
  }
})

test('a PayPal.Me name is letters and digits, and may be pasted as its link', () => {
  expect(paypalName('DanaFixes')).toBe('DanaFixes')
  expect(paypalName('https://www.PayPal.me/DanaFixes/')).toBe('DanaFixes')
  for (const bad of ['dana_fixes', 'a'.repeat(21), 'paypal.me/', 'https://evil.example/x']) {
    expect(paypalName(bad), bad).toBeNull()
  }
})

test('Zelle takes an email address or a US number, shown the way people write one', () => {
  expect(zelleRecipient(' Dana@Example.COM ')).toBe('dana@example.com')
  expect(zelleRecipient('(202) 555-0142')).toBe('+12025550142')
  expect(zelleRecipient('+44 20 7946 0958')).toBeNull()
  expect(zelleRecipient('dana@')).toBeNull()
  expect(zelleShown('+12025550142')).toBe('(202) 555-0142')
  expect(zelleShown('dana@example.com')).toBe('dana@example.com')
})

test('each option is read on its own, and shown to its owner as saved', () => {
  expect(normalizeHandle('venmo', '  ')).toBe('')
  expect(normalizeHandle('venmo', '@dana-fixes')).toBe('dana-fixes')
  expect(normalizeHandle('paypal', 'dana_x')).toBeNull()
  expect(normalizeHandle('zelle', '2025550142')).toBe('+12025550142')
  const saved = { venmo: 'dana-fixes', cash_app: null, paypal: null, zelle: '+12025550142' }
  expect(handleShown('venmo', saved)).toBe('dana-fixes')
  expect(handleShown('zelle', saved)).toBe('(202) 555-0142')
  expect(handleShown('cash_app', saved)).toBe('')
  expect(handleShown('venmo', null)).toBe('')
})

test('the apps added, those left to add, and the account row’s summary', () => {
  const saved = { venmo: 'dana-fixes', cash_app: null, paypal: null, zelle: 'dana@example.com' }
  expect(addedApps(saved)).toEqual(['venmo', 'zelle'])
  expect(appsToAdd(saved)).toEqual(['cash_app', 'paypal'])
  expect(appsToAdd(null)).toEqual(['venmo', 'cash_app', 'paypal', 'zelle'])
  const w = en.payments
  expect(paymentOptionsSummary(saved, w.apps, w.noneAdded)).toBe('Venmo, Zelle')
  expect(paymentOptionsSummary(null, w.apps, w.noneAdded)).toBe('None added')
  expect(
    paymentOptionsSummary({ ...saved, cash_app: 'x', paypal: 'y' }, es.payments.apps, es.payments.noneAdded),
  ).toBe('Venmo, Cash App, PayPal, Zelle')
  expect(appsToAdd({ ...saved, cash_app: 'x', paypal: 'y' })).toEqual([])
})

test('an amount goes in a link as plain dollars, with cents only when there are some', () => {
  expect(linkAmount(10000, 'USD')).toBe('100')
  expect(linkAmount(10050, 'USD')).toBe('100.50')
  expect(linkAmount(5, 'USD')).toBe('0.05')
  expect(linkAmount(123456789, 'USD')).toBe('1234567.89')
  // No other currency, and nothing that is not an amount.
  expect(linkAmount(10000, 'EUR')).toBeNull()
  expect(linkAmount(0, 'USD')).toBeNull()
})

test('the note says the item and the yup, cut short so the reference always fits', () => {
  expect(paymentNote(NOTE, 'Fence deposit', 'SWYK-99A2')).toBe('Fence deposit · yup SWYK-99A2')
  expect(paymentNote(es.payments.note, 'Depósito', 'SWYK-99A2')).toBe('Depósito · yup SWYK-99A2')
  expect(paymentNote(NOTE, '  Two\nlines  ', 'AB')).toBe('Two lines · yup AB')
  const long = paymentNote(NOTE, 'x'.repeat(500), 'SWYK-99A2')
  expect(long.endsWith('… · yup SWYK-99A2')).toBe(true)
  expect([...long].length).toBeLessThanOrEqual(80)
})

test('a Venmo link carries the amount and the note, every character escaped', () => {
  const note = paymentNote(NOTE, 'Rent & deposit #3 + 50% "extra"/€', 'SWYK-99A2')
  const url = venmoUrl('dana-fixes', '100.50', note)
  expect(url).toBe(
    'https://venmo.com/u/dana-fixes?txn=pay&amount=100.50&note=Rent%20%26%20deposit%20%233%20%2B%2050%25%20%22extra%22%2F%E2%82%AC%20%C2%B7%20yup%20SWYK-99A2',
  )
  const parsed = new URL(url)
  expect(parsed.host).toBe('venmo.com')
  expect(parsed.pathname).toBe('/u/dana-fixes')
  expect(parsed.searchParams.get('txn')).toBe('pay')
  expect(parsed.searchParams.get('amount')).toBe('100.50')
  expect(parsed.searchParams.get('note')).toBe(note)
  expect(venmoUrl('dana-fixes', null, null)).toBe('https://venmo.com/u/dana-fixes?txn=pay')
})

test('Cash App and PayPal links carry the amount in the path, and no note', () => {
  expect(cashAppUrl('DanaFixes', '100.50')).toBe('https://cash.app/$DanaFixes/100.50')
  expect(cashAppUrl('DanaFixes', null)).toBe('https://cash.app/$DanaFixes')
  expect(paypalUrl('DanaFixes', '100')).toBe('https://www.paypal.me/DanaFixes/100USD')
  expect(paypalUrl('DanaFixes', null)).toBe('https://www.paypal.me/DanaFixes')
})

test('the sheet offers what the payee saved, in order, and says when it cannot fill the amount', () => {
  const handles = { venmo: 'dana-fixes', cash_app: 'DanaFixes', paypal: 'DanaFixes', zelle: '+12025550142' }
  const note = 'Fence deposit · yup SWYK-99A2'
  const options = payOptions(handles, 10050, 'USD', note)
  expect(options.map((option) => option.app)).toEqual(['venmo', 'cash_app', 'paypal', 'zelle'])
  expect(options[0]).toMatchObject({ amountFilled: true, noteFilled: true })
  expect(options[1]).toMatchObject({ url: 'https://cash.app/$DanaFixes/100.50', noteFilled: false })
  expect(options[3]).toEqual({
    app: 'zelle',
    handle: '+12025550142',
    shown: '(202) 555-0142',
    changedAt: null,
  })
  expect(options.every((option) => option.changedAt === null)).toBe(true)

  // An option the payee changed recently carries when, to warn beside it.
  const changed = payOptions(handles, 10050, 'USD', note, {
    venmo: '2026-10-07T12:00:00Z',
    cash_app: null,
    paypal: null,
    zelle: '2026-10-06T12:00:00Z',
  })
  expect(changed.map((option) => option.changedAt)).toEqual([
    '2026-10-07T12:00:00Z',
    null,
    null,
    '2026-10-06T12:00:00Z',
  ])

  const euros = payOptions(handles, 10050, 'EUR', note)
  expect(euros[0]).toMatchObject({ amountFilled: false })
  expect(euros[1]).toMatchObject({ url: 'https://cash.app/$DanaFixes', amountFilled: false })
  expect(euros[2]).toMatchObject({ url: 'https://www.paypal.me/DanaFixes' })

  expect(payOptions({ venmo: null, cash_app: null, paypal: null, zelle: 'd@x.co' }, 1, 'USD', note)).toHaveLength(1)
  expect(payOptions(null, 1, 'USD', note)).toEqual([])
})

const theirs = { venmo: 'dana-fixes', cash_app: null, paypal: null, zelle: null }
const money = { type: 'MONEY' as const, from: 'B' as const, amount_minor: 10000 }

test('Pay is offered to the payer of money still to be paid, while the payee shows options', () => {
  const exchange = { state: 'ACTIVE' as const, you: 'B' as const, payment_options: { shown: false, theirs } }
  expect(payOffered(exchange, money, 'PENDING')).toBe(true)
  expect(payOffered(exchange, money, 'DISPUTED')).toBe(true)
  for (const status of ['CLAIMED', 'ACCEPTED', 'WAIVED']) {
    expect(payOffered(exchange, money, status), status).toBe(false)
  }
  // Not to the payee, not for goods, not without options, not before it is in force.
  expect(payOffered({ ...exchange, you: 'A' }, money, 'PENDING')).toBe(false)
  expect(payOffered(exchange, { ...money, type: 'SERVICE' as const }, 'PENDING')).toBe(false)
  expect(payOffered({ ...exchange, payment_options: { shown: false, theirs: null } }, money, 'PENDING')).toBe(false)
  expect(payOffered({ ...exchange, payment_options: undefined }, money, 'PENDING')).toBe(false)
  expect(payOffered({ ...exchange, state: 'CLOSED' }, money, 'PENDING')).toBe(false)
})

test('showing options is offered to a party who receives money, and stopping always', () => {
  const terms = { contributions: [{ type: 'MONEY', from: 'B' }] }
  const exchange = {
    state: 'NEGOTIATING' as const,
    you: 'A' as const,
    open_revision: { terms },
    in_force_revision: null,
    payment_options: { shown: false, theirs: null },
  } as unknown as Parameters<typeof showOffered>[0]
  expect(showOffered(exchange)).toBe(true)
  expect(showOffered({ ...exchange, you: 'B' })).toBe(false)
  expect(showOffered({ ...exchange, state: 'DRAFT' })).toBe(false)
  expect(showOffered({ ...exchange, state: 'CLOSED', payment_options: { shown: true, theirs: null } })).toBe(true)
})

test('a change in the options is noticed even when the version is not', () => {
  const on = { payment_options: { shown: false, theirs } }
  const off = { payment_options: { shown: false, theirs: null } }
  expect(paymentOptionsKey(on)).not.toBe(paymentOptionsKey(off))
  expect(paymentOptionsKey(off)).toBe(paymentOptionsKey({ payment_options: { shown: false, theirs: null } }))
})

test('the wording never claims what Yuppers does not do', () => {
  for (const wording of [en.payments, es.payments]) {
    const all = JSON.stringify(wording).toLowerCase()
    for (const claim of ['protected', 'guaranteed', 'escrow', 'secure payment', 'protegid', 'garantizad', 'depósito en garantía', 'pago seguro']) {
      expect(all.includes(claim), claim).toBe(false)
    }
  }
})
