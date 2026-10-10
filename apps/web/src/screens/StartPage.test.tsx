// @vitest-environment jsdom
import { TEMPLATES } from '@yuppers/shared'
import { afterEach, describe, expect, test } from 'vitest'

import { ana, DRAFT } from '../test/fake-service'
import {
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
 * Starting a yup (DESIGN.md §4.4): New yup opens on the common agreements, the
 * blank form and copying an earlier yup; choosing makes the draft, with what
 * the choice puts in it, and opens the composer.
 */

afterEach(stop)

/** What the app asked of the service, in order, for the calls that start a yup. */
function startedBy(sent: { call: string; body: unknown }[]) {
  return sent.filter(
    (request) =>
      request.call === 'POST /v1/exchanges' || request.call === `PUT /v1/exchanges/${DRAFT}/draft`,
  )
}

describe('the chooser', () => {
  test('New yup opens on the common agreements, then blank and copy, and what yups are not for', async () => {
    const { wording } = await start('/', ana)
    const w = wording.templates
    await heading(wording.home.title)
    await until(() => document.querySelector('.cards') !== null, 'the list')
    await press(button(wording.home.start))
    await heading(w.chooserTitle)

    // The six, in the order the design gives, each a button named for it.
    const names = [...document.querySelectorAll('button.choice')].map((found) => found.textContent)
    expect(names).toEqual([
      ...TEMPLATES.map((template) => w.entries[template.id].name),
      w.blank.name,
      w.copy.name,
    ])
    expect(names).toHaveLength(8)
    // The hint is the button's description, in words.
    const first = button(w.entries[TEMPLATES[0].id].name)
    const said = document.getElementById(first.getAttribute('aria-describedby')!)
    expect(said?.textContent).toBe(w.entries[TEMPLATES[0].id].summary)

    // The line about what yups are not for, and that Yuppers is not a lawyer.
    expect(document.body.textContent).toContain(w.notFor)
    expect(document.body.textContent).toContain(w.notForAdvice)
    // New Jersey's warning is on the job and on no other.
    const warnings = [...document.querySelectorAll('.notice-warning')].map((n) => n.textContent)
    expect(warnings).toEqual([w.entries['job-deposit-balance'].warning])
    // Lending money is not among them.
    expect(document.body.textContent).not.toMatch(/loan|lending money|lend money/i)
    expect(await violations()).toEqual([])
  })

  test('is the same in Spanish, with the warning and the line in Spanish', async () => {
    const { wording } = await start('/new', ana, 'es')
    const w = wording.templates
    await heading(w.chooserTitle)
    expect(document.body.textContent).toContain(w.notFor)
    expect(document.body.textContent).toContain(w.entries['job-deposit-balance'].warning)
    expect(document.documentElement.lang).toBe('es')
    expect(await violations()).toEqual([])
  })

  test('a common agreement makes the draft from it and opens it with examples in grey', async () => {
    const { wording, service } = await start('/new', ana)
    const w = wording.templates
    await heading(w.chooserTitle)
    await press(button(w.entries['job-deposit-balance'].name))
    await heading(wording.composer.titleFirst)

    const [created, saved] = startedBy(service.sent)
    // The service is told once what the draft was started from; the working
    // copy has no trace of it.
    expect(created.body).toMatchObject({ started_from: 'job-deposit-balance@1' })
    expect(JSON.stringify(saved.body)).not.toContain('job-deposit-balance')
    const draft = (saved.body as { body: { contributions: Record<string, unknown>[] } }).body
    expect(draft.contributions.map((item) => [item.from, item.type, item.description])).toEqual([
      ['B', 'MONEY', ''],
      ['A', 'SERVICE', ''],
      ['B', 'MONEY', ''],
    ])

    // Three items, each an empty field with its example in grey.
    const descriptions = [...document.querySelectorAll<HTMLTextAreaElement>('textarea')].filter(
      (found) => found.id.endsWith('-description'),
    )
    expect(descriptions.map((found) => found.value)).toEqual(['', '', ''])
    expect(descriptions.map((found) => found.placeholder)).toEqual(
      w.entries['job-deposit-balance'].items.map((item) => item.description),
    )
    // The hint, the grey-text note and the warning are above the items.
    expect(document.body.textContent).toContain(w.entries['job-deposit-balance'].hint)
    expect(document.body.textContent).toContain(w.bandExamples)
    expect(document.body.textContent).toContain(w.entries['job-deposit-balance'].warning)
    expect(await violations()).toEqual([])
  })

  test('an example counts as empty: the composer asks for a description', async () => {
    const { wording } = await start('/new', ana)
    await heading(wording.templates.chooserTitle)
    await press(button(wording.templates.entries['swap-no-money'].name))
    await heading(wording.composer.titleFirst)

    await press(button(wording.composer.review))
    await settle()
    const missing = [...document.querySelectorAll('*')].filter(
      (found) =>
        found.children.length === 0 &&
        found.textContent === wording.composer.problems.DESCRIPTION_MISSING,
    )
    expect(missing).toHaveLength(2)
  })

  test('the hint stays until the first edit, then folds behind a link; the warning stays', async () => {
    const { wording } = await start('/new', ana)
    const w = wording.templates
    await heading(w.chooserTitle)
    await press(button(w.entries['job-deposit-balance'].name))
    await heading(wording.composer.titleFirst)
    const entry = w.entries['job-deposit-balance']
    expect(document.body.textContent).toContain(entry.hint)

    await type(field(wording.composer.termsLabel), 'Two posts and a panel.')
    expect(document.body.textContent).not.toContain(entry.hint)
    expect(document.body.textContent).toContain(entry.warning)
    const link = button(w.bandShow)
    expect(link.getAttribute('aria-expanded')).toBe('false')
    await press(link)
    expect(document.body.textContent).toContain(entry.hint)
    expect(button(w.bandHide).getAttribute('aria-expanded')).toBe('true')
  })

  test('swap sides turns every item round, and announces it', async () => {
    const { wording } = await start('/new', ana)
    const w = wording.templates
    await heading(w.chooserTitle)
    await press(button(w.entries['selling-something'].name))
    await heading(wording.composer.titleFirst)

    const providers = () =>
      [...document.querySelectorAll<HTMLSelectElement>('select')]
        .filter((found) => [...found.options].some((o) => o.text === wording.party.you))
        .map((found) => found.selectedOptions[0].text)
    expect(providers()).toEqual([wording.party.you, wording.party.other])
    await press(button(w.swapSides))
    expect(providers()).toEqual([wording.party.other, wording.party.you])
    await settle()
    expect(document.getElementById('announce-polite')?.textContent).toBe(w.swapped)
    expect(await violations()).toEqual([])
  })

  test('blank is the empty composer, and the service is told so', async () => {
    const { wording, service } = await start('/new', ana)
    await heading(wording.templates.chooserTitle)
    await press(button(wording.templates.blank.name))
    await heading(wording.composer.titleFirst)

    const [created, ...rest] = startedBy(service.sent)
    expect(created.body).toMatchObject({ started_from: 'blank' })
    expect(rest.filter((request) => request.call.startsWith('PUT'))).toHaveLength(0)
    // No band: nothing was chosen to say anything about.
    expect(document.body.textContent).not.toContain(wording.templates.bandExamples)
  })
})

describe('copying a previous yup', () => {
  test('lists the yups that were sent, asks who the copy is for, and copies items and terms', async () => {
    const { wording, service } = await start('/new', ana)
    const w = wording.templates
    await heading(w.chooserTitle)
    await press(button(w.copy.name))
    await heading(w.copyHeading)
    await until(() => document.querySelector('.cards .card') !== null, 'the yups')

    // A draft that was never sent has no terms; the agreement in force does.
    const reference = 'PVVS-5Q2K'
    const cards = [...document.querySelectorAll('.cards .card')]
    expect(cards.length).toBeGreaterThan(0)
    expect(document.body.textContent).toContain(reference)
    expect(document.body.textContent).not.toContain('DRFT-0001')

    const card = cards.find((found) => found.textContent?.includes(reference))!
    await press(card.querySelector('button')!)
    // Someone else is the answer to begin with, so a name never goes by default.
    const someoneElse = field(w.copyForSomeoneElse) as HTMLInputElement
    expect(someoneElse.checked).toBe(true)
    expect(await violations()).toEqual([])

    await press(button(w.copyStart))
    await heading(wording.composer.titleFirst)

    const [created, saved] = startedBy(service.sent)
    expect(created.body).toMatchObject({ started_from: 'copy' })
    const draft = (
      saved.body as {
        body: { partyA: string; partyB: string; terms: string; note: string; contributions: any[] }
      }
    ).body
    expect(draft.partyA).toBe('Ana Ruiz')
    expect(draft.partyB).toBe('')
    expect(draft.terms).toBe('Repair the back fence.')
    expect(draft.note).toBe('')
    expect(draft.contributions.map((item) => item.description)).toEqual([
      'Repair the back fence',
      'Payment for the repair',
    ])
    // The old job’s date is gone; the link between the two items is kept, to the new copies.
    expect(draft.contributions[0].due).toEqual({ kind: 'DATE', date: '' })
    expect(draft.contributions[1].due).toEqual({
      kind: 'AFTER_CONTRIBUTION',
      contribution: draft.contributions[0].id,
    })
    expect(draft.contributions[0].id).not.toBe('11111111-1111-4111-8111-111111111111')
    // Nothing of the old yup is named in what the service was told.
    expect(JSON.stringify(created.body)).not.toContain('PVVS')
  })

  test('for the same person keeps the other party’s name as written', async () => {
    const { wording, service } = await start('/new', ana)
    const w = wording.templates
    await heading(w.chooserTitle)
    await press(button(w.copy.name))
    await until(() => document.querySelector('.cards .card') !== null, 'the yups')
    const card = [...document.querySelectorAll('.cards .card')].find((found) =>
      found.textContent?.includes('PVVS-5Q2K'),
    )!
    await press(card.querySelector('button')!)
    const same = [...document.querySelectorAll<HTMLInputElement>('input[type=radio]')][1]
    await press(same)
    await press(button(w.copyStart))
    await heading(wording.composer.titleFirst)

    const [, saved] = startedBy(service.sent)
    expect((saved.body as { body: { partyB: string } }).body.partyB).toBe('Ben Ortiz')
  })

  test('says so when there is nothing to copy', async () => {
    const { wording } = await start('/new', ana, 'en', (fake) => {
      fake.noExchanges = true
    })
    await heading(wording.templates.chooserTitle)
    await press(button(wording.templates.copy.name))
    await until(
      () => document.body.textContent!.includes(wording.templates.copyNone),
      'the empty list',
    )
    expect(await violations()).toEqual([])
  })
})
