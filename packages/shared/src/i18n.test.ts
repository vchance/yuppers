import type { Account } from '@yuppers/api-client'
import { expect, test } from 'vitest'

import { createI18n, isComplete } from './i18n'
import { wordingFor } from './language'

const en = createI18n('en', wordingFor('en'), () => {})
const es = createI18n('es', wordingFor('es'), () => {})

test('a calendar date is written the way the language writes dates, and stays that date', () => {
  expect(en.day('2026-10-02')).toBe('October 2, 2026')
  expect(es.day('2026-10-02')).toBe('2 de octubre de 2026')
  // Whatever timezone the reader is in, the last day of a year is not the first of the next.
  expect(en.day('2026-12-31')).toBe('December 31, 2026')
  expect(en.day('2027-01-01')).toBe('January 1, 2027')
})

test('something that is not a date or a moment is shown as it came', () => {
  expect(en.day('soon')).toBe('soon')
  expect(en.moment('later')).toBe('later')
})

test('money is written the way the language writes it', () => {
  expect(en.money(40050, 'USD')).toBe('$400.50')
  expect(es.money(40050, 'USD')).toMatch(/^400,50\s(US\$|\$)$/)
})

test('a refusal is told in the reader’s language, and an unknown one as a fault on our side', () => {
  expect(en.errorText('VERSION_CONFLICT')).toBe(wordingFor('en').errors.VERSION_CONFLICT)
  expect(es.errorText('TOO_MANY_REQUESTS')).toBe(wordingFor('es').errors.TOO_MANY_REQUESTS)
  expect(en.errorText('NEWER_THAN_THIS_BUILD' as never)).toBe(wordingFor('en').errors.INTERNAL)
})

test('messages are filled in by the language’s rules', () => {
  expect(en.fmt(wordingFor('en').exchange.remaining, { count: 1 })).toBe(
    '1 required item is still to be confirmed.',
  )
  expect(en.fmt(wordingFor('en').exchange.remaining, { count: 2 })).toBe(
    '2 required items are still to be confirmed.',
  )
})

test('an account can sign once it has a name and has confirmed being an adult', () => {
  const account: Account = { id: 'a', display_name: '', adult_confirmed: false, notification_detail: false, language: 'en' }
  expect(isComplete(account)).toBe(false)
  expect(isComplete({ ...account, display_name: 'Ana' })).toBe(false)
  expect(isComplete({ ...account, adult_confirmed: true })).toBe(false)
  expect(isComplete({ ...account, display_name: 'Ana', adult_confirmed: true })).toBe(true)
})
