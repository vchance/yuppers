// @vitest-environment jsdom
import { afterEach, describe, expect, test } from 'vitest'

import { ana, PANELS, PART_2, PART_3, SERIES } from '../test/fake-service'
import {
  announced,
  button,
  field,
  press,
  settle,
  start,
  stop,
  type,
  until,
  violations,
} from '../test/harness'

/*
 * Instalments and stages once they are agreed (DESIGN.md §7.1, §7.2): where
 * a series stands as counts in words, "Mark the rest as paid", and progress
 * notes that change nothing about an item.
 */

afterEach(stop)

const open = `/exchanges/${SERIES}`

function text(): string {
  return document.body.textContent ?? ''
}

function commandsSent(service: { sent: { call: string; body: unknown }[] }): unknown[] {
  return service.sent
    .filter((request) => request.call === `POST /v1/exchanges/${SERIES}/commands`)
    .map((request) => request.body)
}

describe('counts in words', () => {
  test('say how many payments and stages are confirmed, under each party', async () => {
    await start(open, ana, 'en')
    await until(() => text().includes('Repayment 1 of 3'), 'the agreement')
    const lines = [...document.querySelectorAll('ul.series li')].map((item) => item.textContent)
    expect(lines).toEqual([
      'Ben Ortiz: 1 of 3 payments confirmed',
      'Ana Ruiz: 1 of 2 stages confirmed',
    ])
    // Words, never a bar, an amount or a percentage as status.
    expect(document.querySelector('progress, [role="progressbar"], meter')).toBeNull()
    expect(text()).not.toMatch(/\d\s?%/)
  })

  test('are on the chips of the home list, beside the state', async () => {
    await start('/', ana, 'en', (fake) => {
      fake.series.listed = true
    })
    await until(() => text().includes('1 of 3 payments confirmed'), 'the chips')
    expect(text()).toContain('1 of 2 stages confirmed')
  })
})

describe('mark the rest as paid', () => {
  test('records one payment for each still to pay, after saying what it does', async () => {
    const { service, wording } = await start(open, ana, 'en')
    const w = wording.exchange.markRest
    await until(() => text().includes('Repayment 1 of 3'), 'the agreement')

    await press(button(w.button))
    expect(text()).toContain(
      'This records that you paid Ana Ruiz 2 payments outside our service, one record for each.',
    )
    expect(commandsSent(service)).toEqual([])

    await press(button('Record 2 payments as paid'))
    await settle()
    // One command, naming each payment, in the order the agreement lists them.
    expect(commandsSent(service)).toEqual([
      { expected_version: 3, command: { type: 'CLAIM_REST', contributions: [PART_2, PART_3] } },
    ])
    // Said to a screen reader: the yup was updated.
    expect(announced().polite).not.toBe('')
    // Nothing left to mark, and each payment waits for its own confirmation.
    expect(() => button(w.button)).toThrow()
    expect(text()).toContain('Marked paid, waiting for the receiver to confirm')
  })

  test('is not offered to the party the payments are owed to', async () => {
    const { wording } = await start(open, ana, 'en', (fake) => {
      fake.series.you = 'A'
    })
    await until(() => text().includes('Repayment 1 of 3'), 'the agreement')
    expect(() => button(wording.exchange.markRest.button)).toThrow()
  })
})

describe('progress notes', () => {
  test('are written by the provider, kept as their own words, and change no status', async () => {
    const { service, wording } = await start(open, ana, 'en', (fake) => {
      fake.series.you = 'A'
    })
    const w = wording.exchange.progress
    await until(() => text().includes('Panels up'), 'the agreement')
    // One item is under way and Ana provides it: Posts set is already confirmed.
    const add = [...document.querySelectorAll('button')].filter(
      (candidate) => candidate.textContent === w.add,
    )
    expect(add).toHaveLength(1)

    await press(add[0])
    // Said before it is written: both can read it, and it is not a delivery.
    expect(text()).toContain('Both of you can read it')
    expect(text()).toContain('There is no text message.')
    await press(button(w.send))
    expect(text()).toContain(w.required)
    expect(commandsSent(service)).toEqual([])

    await type(field(w.label), 'Panels go up on Thursday.')
    await press(button(w.send))
    await settle()
    expect(commandsSent(service)).toEqual([
      {
        expected_version: 3,
        command: { type: 'NOTE_PROGRESS', contribution: PANELS, note: 'Panels go up on Thursday.' },
      },
    ])
    expect(announced().polite).not.toBe('')
    // It shows under the item, and the item stays what it was.
    await until(() => text().includes('1 progress note'), 'the note')
    expect(text()).toContain('Panels go up on Thursday.')
    expect(text()).toContain('Not delivered yet')
  })

  test('are for the provider only, and say so when an item is full', async () => {
    const { wording } = await start(open, ana, 'en', (fake) => {
      fake.series.you = 'A'
      fake.series.noteLimit = 1
    })
    const w = wording.exchange.progress
    await until(() => text().includes('Panels up'), 'the agreement')
    await press(button(w.add))
    await type(field(w.label), 'One.')
    await press(button(w.send))
    await until(() => text().includes('1 progress note'), 'the note')

    await press(button(w.add))
    await type(field(w.label), 'Two.')
    await press(button(w.send))
    await until(() => text().includes(w.full), 'the refusal')
  })

  test('are offered on the payments a party still owes, not on those they receive', async () => {
    const { wording } = await start(open, ana, 'en')
    await until(() => text().includes('Repayment 1 of 3'), 'the agreement')
    const add = [...document.querySelectorAll('button')].filter(
      (candidate) => candidate.textContent === wording.exchange.progress.add,
    )
    // Two payments still to pay; the first is confirmed and the stages are Ana's.
    expect(add).toHaveLength(2)
  })

  test('have no accessibility violations, panel open', async () => {
    const { wording } = await start(open, ana, 'en', (fake) => {
      fake.series.you = 'A'
    })
    await until(() => text().includes('Panels up'), 'the agreement')
    await press(button(wording.exchange.progress.add))
    await settle()
    expect(await violations()).toEqual([])
  })
})
