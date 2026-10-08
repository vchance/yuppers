import { describe, expect, test } from 'vitest'

import {
  formatPhone,
  identifierToSend,
  maskPhone,
  phoneAsTyped,
  phoneProblem,
  readPhone,
  usPhone,
} from './phone'

/*
 * Phone numbers as people in the US write them, read the same way the
 * service reads them (`Identifier::parse` in backend/src/domain/identity.rs).
 */

/** Every way of writing a number that is taken, and what it becomes. */
const ACCEPTED: [typed: string, e164: string][] = [
  ['8565488780', '+18565488780'],
  ['856-548-8780', '+18565488780'],
  ['(856) 548-8780', '+18565488780'],
  ['856.548.8780', '+18565488780'],
  ['856 548 8780', '+18565488780'],
  ['1 856 548 8780', '+18565488780'],
  ['1-856-548-8780', '+18565488780'],
  ['18565488780', '+18565488780'],
  ['+1 856 548 8780', '+18565488780'],
  ['+18565488780', '+18565488780'],
  ['+1 (856) 548-8780', '+18565488780'],
  ['  (856)548-8780  ', '+18565488780'],
  ['202-555-0142', '+12025550142'],
]

/** What is refused, and why: `invalid`, or `country` where only +1 is texted. */
const REFUSED: [typed: string, problem: 'invalid' | 'country'][] = [
  // Another country code: not served.
  ['+44 20 7946 0958', 'country'],
  ['+52 55 1234 5678', 'country'],
  ['+33612345678', 'country'],
  // Too few or too many digits.
  ['548-8780', 'invalid'],
  ['856548878', 'invalid'],
  ['85654887801', 'invalid'],
  ['2 856 548 8780', 'invalid'],
  ['+1 856 548 878', 'invalid'],
  ['+1 856 548 87801', 'invalid'],
  // An area code or exchange starting with 0 or 1.
  ['056 548 8780', 'invalid'],
  ['156 548 8780', 'invalid'],
  ['856 048 8780', 'invalid'],
  ['856 148 8780', 'invalid'],
  ['1 856 148 8780', 'invalid'],
  ['+1 156 548 8780', 'invalid'],
  // Not a number.
  ['', 'invalid'],
  ['call me', 'invalid'],
  ['856 548 8780 ext 2', 'invalid'],
  ['856+548+8780', 'invalid'],
  ['++18565488780', 'invalid'],
  ['+0123456789', 'invalid'],
  ['ana@example.test', 'invalid'],
]

describe('a phone number', () => {
  test.each(ACCEPTED)('%s is taken as %s', (typed, e164) => {
    expect(readPhone(typed)).toEqual({ kind: 'nanp', phone: e164 })
    expect(usPhone(typed)).toBe(e164)
    expect(phoneProblem(typed, ['+1'])).toBeNull()
    expect(identifierToSend(typed)).toBe(e164)
  })

  test.each(REFUSED)('%s is refused (%s)', (typed, problem) => {
    expect(usPhone(typed)).toBeNull()
    expect(phoneProblem(typed, ['+1'])).toBe(problem)
  })

  test('of another country is read as that country’s, for the service to decide', () => {
    expect(readPhone('+44 20 7946 0958')).toEqual({ kind: 'international', phone: '+442079460958' })
    // A service that texts it says so; one that has not said decides itself.
    expect(phoneProblem('+44 20 7946 0958', ['+1', '+44'])).toBeNull()
    expect(phoneProblem('+44 20 7946 0958', [])).toBeNull()
    expect(identifierToSend('+44 20 7946 0958')).toBe('+442079460958')
  })

  test('is sent in E.164, and anything else as typed, trimmed', () => {
    expect(identifierToSend(' Ana@Example.test ')).toBe('Ana@Example.test')
    expect(identifierToSend(' 856 548 ')).toBe('856 548')
  })

  test('is shown the American way, in full or with all but its last four digits hidden', () => {
    expect(formatPhone('+18565488780')).toBe('(856) 548-8780')
    expect(formatPhone('+442079460958')).toBe('+442079460958')
    expect(formatPhone('ana@example.test')).toBe('ana@example.test')
    expect(maskPhone('+18565488780')).toBe('(•••) •••-8780')
    expect(maskPhone('+447700900123')).toBe('•••0123')
  })

  test('is written the American way once typed, and anything else left to correct', () => {
    expect(phoneAsTyped('8565488780')).toBe('(856) 548-8780')
    expect(phoneAsTyped('+1 856 548 8780')).toBe('(856) 548-8780')
    expect(phoneAsTyped('(856) 548-8780')).toBe('(856) 548-8780')
    expect(phoneAsTyped('856 548')).toBe('856 548')
    expect(phoneAsTyped('+44 20 7946 0958')).toBe('+44 20 7946 0958')
    expect(phoneAsTyped('ana@example.test')).toBe('ana@example.test')
  })
})
