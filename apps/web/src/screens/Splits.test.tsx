// @vitest-environment jsdom
import { wordingFor } from '@yuppers/shared'
import { afterEach, describe, expect, test } from 'vitest'

import { ana, DRAFT } from '../test/fake-service'
import {
  announced,
  button,
  field,
  heading,
  press,
  settle,
  start,
  stop,
  type,
  until,
  violations,
} from '../test/harness'

/*
 * The composer's split sheets (DESIGN.md §7.1, §7.2): a money item becomes
 * instalments, a service or task item becomes stages. The sheet shows every
 * amount and every date before it adds anything, what it adds is ordinary
 * items, and it can be put back while nothing has been sent.
 */

afterEach(stop)

function descriptions(): string[] {
  return [...document.querySelectorAll<HTMLTextAreaElement>('textarea[id$="-description"]')].map(
    (area) => area.value,
  )
}

function previewLines(): string[] {
  return [...document.querySelectorAll('#split-preview + ol li')].map(
    (item) => item.textContent ?? '',
  )
}

describe('splitting a payment into instalments', () => {
  test('shows every amount and date first, then adds plain payments, in order', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    const w = wording.composer.split
    await heading(wording.composer.titleFirst)

    await press(button(w.instalments.link))
    // The sheet opens at three payments, shared out from the amount already typed.
    expect((field(w.instalments.countLabel) as HTMLInputElement).value).toBe('3')
    await type(field('Amount to share out, in USD'), '100')
    await settle()

    // $100 in three reads $33.33, $33.33, $33.34, each with its date, before anything is added.
    const lines = previewLines()
    expect(lines).toHaveLength(3)
    expect(lines[0]).toMatch(/^Payment for the repair 1 of 3: \$33\.33, due /)
    expect(lines[1]).toMatch(/^Payment for the repair 2 of 3: \$33\.33, due /)
    expect(lines[2]).toMatch(/^Payment for the repair 3 of 3: \$33\.34, due /)
    expect(descriptions()).toEqual(['Repair the back fence', 'Payment for the repair'])

    await press(button(w.instalments.done))
    await settle()
    expect(descriptions()).toEqual([
      'Repair the back fence',
      'Payment for the repair 1 of 3',
      'Payment for the repair 2 of 3',
      'Payment for the repair 3 of 3',
    ])
    expect(announced().polite).toContain('3 payments added.')
    // Plain money items from the same payer: each an amount and a date, nothing else added.
    const added = [...document.querySelectorAll('fieldset')].filter((item) =>
      item.querySelector('legend')?.textContent?.startsWith('Item'),
    )
    expect(added).toHaveLength(4)
    for (const item of added.slice(1)) {
      expect(item.querySelector<HTMLSelectElement>('select')?.value).toBe('B')
    }
    const text = document.body.textContent ?? ''
    for (const word of ['interest', 'late fee', 'balance', 'rate']) {
      expect(text.toLowerCase()).not.toContain(word)
    }
  })

  test('can be put back as one payment while nothing has been sent', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    const w = wording.composer.split.instalments
    await heading(wording.composer.titleFirst)
    await press(button(w.link))
    await press(button(w.done))
    await settle()
    expect(descriptions()).toHaveLength(4)

    await press(button(w.undo))
    await settle()
    expect(descriptions()).toEqual(['Repair the back fence', 'Payment for the repair'])
    expect(announced().polite).toContain(w.undone)
    // Back to the amount it was, summed from the parts.
    const moneyInput = document.querySelector<HTMLInputElement>(`input[id$="-amount"]`)!
    expect(moneyInput.value).toBe('450.00')
  })

  test('says what is wrong with the number of payments, and keeps the sheet', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    const w = wording.composer.split.instalments
    await heading(wording.composer.titleFirst)
    await press(button(w.link))
    await type(field(w.countLabel), '40')
    await press(button(w.done))
    await settle()
    expect(document.body.textContent).toContain('Enter a whole number from 2 to 12.')
    expect(descriptions()).toHaveLength(2)
    expect(document.activeElement).toBe(field(w.countLabel))
  })

  test('is a sheet without accessibility violations', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    await heading(wording.composer.titleFirst)
    await press(button(wording.composer.split.instalments.link))
    await settle()
    expect(await violations()).toEqual([])
  })

  test('says it in Spanish: “Pago 1 de 3”, for the author’s own description', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    await heading(wording.composer.titleFirst)
    await type(document.querySelector<HTMLSelectElement>('.language select')!, 'es')
    await until(() => document.documentElement.lang === 'es', 'Spanish')
    const es = wordingFor('es').composer.split
    await press(button(es.instalments.link))
    await press(button(es.instalments.done))
    await settle()
    // Written once, in the language the author is using, from their own words.
    expect(descriptions()).toContain('Payment for the repair 1 de 3')
    expect(announced().polite).toContain('3 pagos agregados.')
  })
})

describe('splitting a job into stages', () => {
  test('names each stage, adds a payment for each if asked, and pairs them', async () => {
    const { wording, service } = await start(`/exchanges/${DRAFT}`, ana)
    const w = wording.composer.split.stages
    await heading(wording.composer.titleFirst)

    await press(button(w.link))
    // Unchained and unpaid until ticked.
    expect((field(w.chainLabel) as HTMLInputElement).checked).toBe(false)
    expect((field(w.payLabel) as HTMLInputElement).checked).toBe(false)
    // Examples are hint text under the label, not placeholders.
    expect(field('Name of stage 1').getAttribute('placeholder')).toBeNull()
    expect(document.body.textContent).toContain(w.exampleOne)

    await type(field('Name of stage 1'), 'Posts set')
    await type(field('Name of stage 2'), 'Panels up')
    await type(field('Name of stage 3'), 'Painted')
    await press(field(w.payLabel))
    await press(field(wording.composer.split.amounts.howEach))
    await type(field('Amount of each, in USD'), '100')
    await settle()
    expect(
      [...document.querySelectorAll('#split-preview + ol li')].map((item) => item.textContent),
    ).toContain('Payment for Posts set: $100.00, due once stage 1 is confirmed')

    await press(button(w.done))
    await settle()
    // Each stage is followed by its own payment.
    expect(descriptions()).toEqual([
      'Posts set',
      'Payment for Posts set',
      'Panels up',
      'Payment for Panels up',
      'Painted',
      'Payment for Painted',
      'Payment for the repair',
    ])
    expect(announced().polite).toContain('6 items added.')

    // The working copy remembers how to put it back, and nothing else about it.
    await until(
      () =>
        service.sent.some(
          (request) => request.call === `PUT /v1/exchanges/${DRAFT}/draft` && JSON.stringify(request.body).includes('STAGES'),
        ),
      'the saved draft',
    )
  })
})
