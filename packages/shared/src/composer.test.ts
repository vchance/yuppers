import type { ExchangeView, RevisionTerms } from '@yuppers/api-client'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'

import type { RevisionView } from './api'
import {
  baseRevision,
  canCompose,
  composerKind,
  createDraftSaver,
  dueOf,
  lockedContributions,
  problemText,
  revisionToSend,
  startingDraft,
  type SaveState,
} from './composer'
import { CONSENT_VERSION } from './consent'
import { buildTerms, emptyDraft, newContribution, type Draft } from './draft'
import { wordingFor } from './language'

const REPAIR = '11111111-1111-4111-8111-111111111111'
const PAYMENT = '22222222-2222-4222-8222-222222222222'

const terms: RevisionTerms = {
  party_a_name: 'Ana Ruiz',
  party_b_name: 'Ben Ortiz',
  terms: 'Repair the back fence.',
  contributions: [
    {
      id: REPAIR,
      from: 'A',
      type: 'SERVICE',
      description: 'Repair the back fence',
      due: { kind: 'ON_AGREEMENT' },
      required: true,
    },
    {
      id: PAYMENT,
      from: 'B',
      type: 'MONEY',
      description: 'Payment for the repair',
      due: { kind: 'AFTER_CONTRIBUTION', contribution: REPAIR },
      required: true,
      amount_minor: 40000,
    },
  ],
}

function revision(id: string): RevisionView {
  return {
    id,
    sequence: 1,
    author: 'A',
    accepted_by: ['A'],
    content_hash: 'ab'.repeat(32),
    expires_at: '2026-10-16T12:00:00Z',
    terms,
  }
}

function exchange(patch: Partial<ExchangeView>): ExchangeView {
  return {
    id: '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70',
    version: 3,
    state: 'DRAFT',
    you: 'A',
    counterparty: 'UNCLAIMED',
    currency: 'USD',
    timezone: 'America/Chicago',
    display_code: 'ABCD-1234',
    contributions: [],
    ...patch,
  }
}

const draft = exchange({})
const negotiating = exchange({ state: 'NEGOTIATING', open_revision: revision('open') })
const active = exchange({
  state: 'ACTIVE',
  in_force_revision: revision('in-force'),
  contributions: [
    { id: REPAIR, status: 'ACCEPTED' },
    { id: PAYMENT, status: 'CLAIMED' },
  ],
})

describe('what a composer starts from', () => {
  test('the three kinds differ only in where they start', () => {
    expect(composerKind(draft)).toBe('first')
    expect(composerKind(negotiating)).toBe('counter')
    expect(composerKind(active)).toBe('amend')
    expect(baseRevision(draft)).toBeNull()
    expect(baseRevision(negotiating)?.id).toBe('open')
    expect(baseRevision(active)?.id).toBe('in-force')
    // An amendment already on the table is what the next one answers.
    expect(baseRevision({ ...active, open_revision: revision('amendment') })?.id).toBe('amendment')
  })

  test('nothing can be composed on a closed exchange', () => {
    expect(canCompose(draft)).toBe(true)
    expect(canCompose(negotiating)).toBe(true)
    expect(canCompose(active)).toBe(true)
    expect(canCompose(exchange({ state: 'CLOSED', in_force_revision: revision('x') }))).toBe(false)
    expect(canCompose(exchange({ state: 'NEGOTIATING' }))).toBe(false)
  })

  test('a first proposal starts with the author’s name and nothing else', () => {
    expect(startingDraft(draft, 'Ana Ruiz', 2)).toEqual(emptyDraft('Ana Ruiz'))
  })

  test('a counteroffer starts from the terms on the table', () => {
    const started = startingDraft(negotiating, 'Ben Ortiz', 2)
    expect(started.base).toBe('open')
    expect(started.partyA).toBe('Ana Ruiz')
    expect(started.contributions.map((item) => item.id)).toEqual([REPAIR, PAYMENT])
    expect(started.contributions[1].amount).toBe('400.00')
  })

  test('a working copy the service stored wins over both', () => {
    const stored = { ...emptyDraft('Ana'), partyB: 'Someone typed this' }
    const started = startingDraft({ ...negotiating, draft: stored as never }, 'Ana', 2)
    expect(started.partyB).toBe('Someone typed this')
  })

  test('only an agreement in force locks what has been accepted', () => {
    expect([...lockedContributions(active)]).toEqual([REPAIR])
    expect([...lockedContributions(negotiating)]).toEqual([])
  })

  test('choosing a kind of due condition starts it empty', () => {
    expect(dueOf('ON_AGREEMENT')).toEqual({ kind: 'ON_AGREEMENT' })
    expect(dueOf('DATE')).toEqual({ kind: 'DATE', date: '' })
    expect(dueOf('AFTER_CONTRIBUTION')).toEqual({ kind: 'AFTER_CONTRIBUTION', contribution: '' })
  })
})

test('a problem is explained in the reader’s language, with numbers as that language writes them', () => {
  const en = wordingFor('en')
  const es = wordingFor('es')
  expect(problemText({ code: 'DESCRIPTION_MISSING', field: 'description' }, en, 'en')).toBe(
    en.composer.problems.DESCRIPTION_MISSING,
  )
  expect(problemText({ code: 'NOTE_TOO_LONG', field: 'note' }, en, 'en')).toContain('1,000')
  expect(problemText({ code: 'QUANTITY_INVALID', field: 'quantity' }, en, 'en')).toContain('1.5')
  expect(problemText({ code: 'QUANTITY_INVALID', field: 'quantity' }, es, 'es')).toContain('1,5')
  expect(problemText({ code: 'AMOUNT_INVALID', field: 'amount' }, es, 'es')).toContain('25,50')
})

describe('the revision to send', () => {
  function built(note = '') {
    const working: Draft = {
      ...emptyDraft('Ana Ruiz'),
      partyB: 'Ben Ortiz',
      note,
      contributions: [{ ...newContribution(REPAIR, 'A'), description: 'Repair the fence' }],
    }
    const result = buildTerms(working, 2)
    if (!result.ok) throw new Error('the working copy should build')
    return result
  }

  test('names the version it was based on and the consent wording shown', () => {
    const body = revisionToSend(draft, built('See you Friday'), 'es', '')
    expect(body.expected_version).toBe(3)
    expect(body.consent).toEqual({ language: 'es', version: CONSENT_VERSION })
    expect(body.note).toBe('See you Friday')
    expect(body.terms.party_b_name).toBe('Ben Ortiz')
  })

  test('a first proposal issues the invitation, bound to someone only if named', () => {
    expect(revisionToSend(draft, built(), 'en', '').invitation).toEqual({ for_anyone: true })
    expect(revisionToSend(draft, built(), 'en', '  ').invitation).toEqual({ for_anyone: true })
    expect(revisionToSend(draft, built(), 'en', ' ben@example.test ').invitation).toEqual({
      bound_to: 'ben@example.test',
    })
  })

  test('a counteroffer or an amendment issues none', () => {
    expect(revisionToSend(negotiating, built(), 'en', 'ben@example.test').invitation).toBeNull()
    expect(revisionToSend(active, built(), 'en', '').invitation).toBeNull()
  })
})

describe('saving a working copy as it is typed', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  const copy = (terms: string): Draft => ({ ...emptyDraft('Ana'), terms })

  /** A saver whose saves are finished by hand, to control what overlaps. */
  function setup() {
    const saved: string[] = []
    const states: SaveState[] = []
    const pending: { resolve(): void; reject(error: Error): void }[] = []
    const saver = createDraftSaver({
      save: (draft) => {
        saved.push(draft.terms)
        return new Promise<void>((resolve, reject) => pending.push({ resolve, reject }))
      },
      onState: (state) => states.push(state),
      delayMs: 700,
    })
    const finish = async (how: 'ok' | 'fail' = 'ok') => {
      const next = pending.shift()
      if (!next) throw new Error('no save is on its way')
      if (how === 'ok') next.resolve()
      else next.reject(new Error('offline'))
      await vi.advanceTimersByTimeAsync(0)
    }
    return { saver, saved, states, finish }
  }

  test('saves once the typing pauses, not on every keystroke', async () => {
    const { saver, saved, states, finish } = setup()
    saver.changed(copy('a'))
    await vi.advanceTimersByTimeAsync(600)
    saver.changed(copy('ab'))
    await vi.advanceTimersByTimeAsync(600)
    expect(saved).toEqual([])
    await vi.advanceTimersByTimeAsync(100)
    expect(saved).toEqual(['ab'])
    expect(states).toEqual(['saving'])
    await finish()
    expect(states).toEqual(['saving', 'saved'])
  })

  test('one save at a time: what was typed meanwhile goes in the next one', async () => {
    const { saver, saved, states, finish } = setup()
    saver.changed(copy('first'))
    await vi.advanceTimersByTimeAsync(700)
    saver.changed(copy('second'))
    await vi.advanceTimersByTimeAsync(700)
    // The first save has not come back, so the second has not left.
    expect(saved).toEqual(['first'])
    await finish()
    await vi.advanceTimersByTimeAsync(700)
    expect(saved).toEqual(['first', 'second'])
    await finish()
    expect(states.at(-1)).toBe('saved')
  })

  test('a failed save is said so and tried again with the next change', async () => {
    const { saver, saved, states, finish } = setup()
    saver.changed(copy('kept'))
    await vi.advanceTimersByTimeAsync(700)
    await finish('fail')
    expect(states).toEqual(['saving', 'failed'])
    saver.changed(copy('kept, and more'))
    await vi.advanceTimersByTimeAsync(700)
    expect(saved).toEqual(['kept', 'kept, and more'])
  })

  test('before sending, a waiting save is dropped and one in flight is waited for', async () => {
    const { saver, saved, finish } = setup()
    saver.changed(copy('in flight'))
    await vi.advanceTimersByTimeAsync(700)
    saver.changed(copy('waiting'))

    let settled = false
    void saver.settle().then(() => (settled = true))
    await vi.advanceTimersByTimeAsync(5000)
    expect(settled).toBe(false)
    await finish()
    expect(settled).toBe(true)

    saver.sent()
    saver.changed(copy('after sending'))
    await vi.advanceTimersByTimeAsync(5000)
    saver.leave()
    expect(saved).toEqual(['in flight'])
  })

  test('when sending fails, the working copy is still saved on the way out', async () => {
    const { saver, saved } = setup()
    saver.changed(copy('unsent'))
    await saver.settle()
    saver.leave()
    expect(saved).toEqual([])
    saver.resume()
    saver.leave()
    expect(saved).toEqual(['unsent'])
  })

  test('leaving saves what was typed in the last moment, and only if something was', async () => {
    const { saver, saved, finish } = setup()
    saver.leave()
    expect(saved).toEqual([])
    saver.changed(copy('last words'))
    saver.leave()
    expect(saved).toEqual(['last words'])
    await finish()
    // The timer that was waiting does not fire a second save.
    await vi.advanceTimersByTimeAsync(5000)
    expect(saved).toEqual(['last words'])
  })
})
