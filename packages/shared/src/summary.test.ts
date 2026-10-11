import type { components } from '@yuppers/api-client'
import { describe, expect, test } from 'vitest'

import { createI18n } from './i18n'
import { wordingFor } from './language'
import type { RecordDocument, RecordEvent, RecordRevision } from './record'
import { recordDays } from './record'
import { summarizeRecord, summaryText } from './summary'

type Status = components['schemas']['Status']

const BIKE = '00000000-0000-4000-8000-000000000011'
const PAY = '00000000-0000-4000-8000-000000000012'
const HELMET = '00000000-0000-4000-8000-000000000013'
const V1 = 'c0000000-0000-4000-8000-000000000001'
const V2 = 'c0000000-0000-4000-8000-000000000002'
const PARTIES = { A: 'Ana Ruiz', B: 'Ben Ortiz' }

function version(
  id: string,
  sequence: number,
  standing: RecordRevision['standing'],
  signers: ('A' | 'B')[],
  contributions = [
    { id: BIKE, from: 'A' as const, type: 'ITEM' as const, description: 'A blue bicycle' },
    {
      id: PAY,
      from: 'B' as const,
      type: 'MONEY' as const,
      description: 'Payment for the bicycle',
      amount_minor: 12000,
    },
  ],
): RecordRevision {
  return {
    id,
    sequence,
    author: 'A',
    sent_at: '2026-10-02T15:00:00Z',
    expires_at: '2026-10-16T15:00:00Z',
    content_hash: 'ab'.repeat(32),
    standing,
    signatures: signers.map((party, index) => ({
      party,
      name: PARTIES[party],
      signed_at: `2026-10-0${2 + index}T16:00:00Z`,
      content_hash: 'ab'.repeat(32),
      consent: { language: 'en', version: '1' },
      verification: { method: 'EMAIL_OTP', verified_at: '2026-10-02T15:00:00Z', description: '' },
    })),
    signed: {
      v: 1,
      exchange: 'e',
      currency: 'USD',
      timezone: 'America/Chicago',
      parties: PARTIES,
      terms: '',
      attachments: [],
      contributions: contributions.map((item) => ({
        due: { kind: 'ON_AGREEMENT' as const },
        required: true,
        quantity: null,
        completion_criteria: null,
        amount_minor: null,
        ...item,
      })),
    },
  }
}

interface Shape {
  state: RecordDocument['exchange']['state']
  outcome?: RecordDocument['exchange']['closed_outcome']
  reason?: string
  revisions: RecordRevision[]
  inForce?: string
  statuses?: [string, Status][]
  events?: Omit<RecordEvent, 'sequence' | 'at'>[]
}

function recordOf(shape: Shape): RecordDocument {
  const inForce = shape.revisions.find((revision) => revision.id === shape.inForce)
  return {
    format: 'exchange-record',
    format_version: 2,
    generated_at: '2026-10-30T12:00:00Z',
    language: 'en',
    notices: { about: '', signatures: '', statements: '', content_hash: '' },
    prepared_for: 'A',
    parties: PARTIES,
    exchange: {
      id: 'e',
      display_code: 'TEST-0001',
      timezone: 'America/Chicago',
      currency: 'USD',
      created_at: '2026-10-02T14:00:00Z',
      state: shape.state,
      counterparty: 'CONFIRMED',
      closed_outcome: shape.outcome ?? null,
      closed_reason: shape.reason ?? null,
      closed_at: shape.state === 'CLOSED' ? '2026-10-20T18:00:00Z' : null,
      in_force_revision: inForce ? { id: inForce.id, sequence: inForce.sequence } : null,
      last_event: 1,
    },
    contributions: (shape.statuses ?? []).map(([id, status]) => ({
      id,
      from: id === PAY ? 'B' : 'A',
      description: '',
      required: true,
      status,
    })),
    revisions: shape.revisions,
    events: (shape.events ?? []).map((event, index) => ({
      sequence: index + 1,
      at: '2026-10-10T12:00:00Z',
      ...event,
    })),
    part: { from: { revisions_after: 0, events_after: 0 }, next: null, complete: true },
  }
}

const inForceStanding = {
  status: 'IN_FORCE' as const,
  since: '2026-10-03T16:00:00Z',
  in_force_at: '2026-10-03T16:00:00Z',
}

const en = createI18n('en', wordingFor('en'), () => {})
const es = createI18n('es', wordingFor('es'), () => {})
const day = recordDays('en', 'America/Chicago')
const say = (record: RecordDocument, i18n = en) =>
  summaryText(summarizeRecord(record), i18n, 'USD', recordDays(i18n.language, 'America/Chicago'))

describe('an agreement in force', () => {
  const record = recordOf({
    state: 'ACTIVE',
    revisions: [version(V1, 1, inForceStanding, ['A', 'B'])],
    inForce: V1,
    statuses: [
      [BIKE, 'CLAIMED'],
      [PAY, 'PENDING'],
    ],
  })

  test('reads its items from the agreement and says where each stands', () => {
    const summary = summarizeRecord(record)
    expect(summary.basis).toEqual({ kind: 'AGREEMENT', sequence: 1 })
    expect(summary.items.map((item) => [item.description, item.outcome, item.by])).toEqual([
      ['A blue bicycle', 'CLAIMED', 'A'],
      ['Payment for the bicycle', 'OUTSTANDING', null],
    ])
    expect(summary.signatures.map((signature) => signature.party)).toEqual(['A', 'B'])
    expect(summary.unsigned).toEqual([])
  })

  test('in plain words', () => {
    const text = say(record)
    expect(text.between).toBe('Between Ana Ruiz and Ben Ortiz.')
    expect(text.basis).toBe('From version 1, the last version both parties signed.')
    expect(text.sides.map((side) => side.heading)).toEqual([
      'What Ana Ruiz agreed to give',
      'What Ben Ortiz agreed to give',
    ])
    expect(text.sides[0].items[0].outcome).toBe(
      'Ana Ruiz marked it delivered. Ben Ortiz has not confirmed it.',
    )
    // Money is spoken of as paying, with its amount.
    expect(text.sides[1].items[0].outcome).toBe('Not paid yet.')
    expect(text.sides[1].items[0].details).toEqual(['Amount: $120.00'])
    expect(text.signed).toEqual([
      `Ana Ruiz signed it on ${day('2026-10-02T16:00:00Z')}.`,
      `Ben Ortiz signed it on ${day('2026-10-03T16:00:00Z')}.`,
      `It came into force on ${day('2026-10-03T16:00:00Z')}.`,
    ])
    expect(text.standing).toEqual([en.wording.record.summary.standing.ACTIVE])
  })

  test('a dispute and a waiver name who did them', () => {
    const disputed = recordOf({
      state: 'ACTIVE',
      revisions: [version(V1, 1, inForceStanding, ['A', 'B'])],
      inForce: V1,
      statuses: [
        [BIKE, 'DISPUTED'],
        [PAY, 'WAIVED'],
      ],
      events: [
        { type: 'CONTRIBUTION_DISPUTED', actor: 'B', contribution: { id: BIKE, description: '' } },
        { type: 'CONTRIBUTION_WAIVED', actor: 'A', contribution: { id: PAY, description: '' } },
      ],
    })
    const text = say(disputed)
    expect(text.sides[0].items[0].outcome).toBe(
      'Ben Ortiz disputed it. What each of them said is in the history below.',
    )
    expect(text.sides[1].items[0].outcome).toBe(
      'Ana Ruiz waived it, releasing Ben Ortiz from paying it.',
    )
  })
})

test('closed without ever being agreed: what the last version proposed, and no outcomes', () => {
  const record = recordOf({
    state: 'CLOSED',
    outcome: 'NOT_AGREED',
    reason: 'DECLINED',
    revisions: [
      version(V1, 1, { status: 'SUPERSEDED', since: '2026-10-04T00:00:00Z' }, ['A']),
      version(V2, 2, { status: 'DECLINED', since: '2026-10-05T00:00:00Z' }, ['A']),
    ],
  })
  const summary = summarizeRecord(record)
  expect(summary.basis).toEqual({ kind: 'LAST', sequence: 2 })
  expect(summary.items.every((item) => item.outcome === null)).toBe(true)
  expect(summary.unsigned).toEqual(['B'])
  expect(summary.inForceAt).toBeNull()

  const text = say(record)
  expect(text.basis).toBe(
    'Nothing was ever signed by both parties. This is what the last version, number 2, proposed.',
  )
  expect(text.sides[0].heading).toBe('What Ana Ruiz would have given')
  expect(text.sides[0].items[0].outcome).toBeNull()
  expect(text.signed).toEqual([
    `Ana Ruiz signed it on ${day('2026-10-02T16:00:00Z')}.`,
    'Ben Ortiz did not sign it.',
  ])
  expect(text.standing).toEqual([
    `It closed on ${day('2026-10-20T18:00:00Z')} without an agreement, so neither of them was bound to anything.`,
    en.wording.record.closedReasons.DECLINED,
  ])
})

test('closed unresolved: each item as it was left, and why it closed', () => {
  const record = recordOf({
    state: 'CLOSED',
    outcome: 'UNRESOLVED',
    reason: 'CLOSE_REQUEST',
    revisions: [version(V1, 1, inForceStanding, ['A', 'B'])],
    inForce: V1,
    statuses: [
      [BIKE, 'ACCEPTED'],
      [PAY, 'PENDING'],
    ],
    events: [
      { type: 'CONTRIBUTION_CONFIRMED', actor: 'B', contribution: { id: BIKE, description: '' } },
    ],
  })
  const text = say(record)
  expect(text.sides[0].items[0].outcome).toBe('Delivered, and Ben Ortiz confirmed receiving it.')
  expect(text.sides[1].items[0].outcome).toBe('Never recorded as paid before it closed.')
  expect(text.standing).toEqual([
    `It closed on ${day('2026-10-20T18:00:00Z')} without agreement, as unresolved. Nobody was released: each item kept the status it had.`,
    en.wording.record.closedReasons.CLOSE_REQUEST,
  ])
})

test('ended by agreement: who proposed it, and what was released by it', () => {
  const record = recordOf({
    state: 'CLOSED',
    outcome: 'ENDED_BY_AGREEMENT',
    revisions: [version(V1, 1, inForceStanding, ['A', 'B'])],
    inForce: V1,
    statuses: [
      [BIKE, 'ACCEPTED'],
      [PAY, 'WAIVED'],
    ],
    events: [
      { type: 'CONTRIBUTION_CONFIRMED', actor: 'B', contribution: { id: BIKE, description: '' } },
      { type: 'END_PROPOSED', actor: 'B' },
      { type: 'EXCHANGE_CLOSED', actor: 'A', outcome: 'ENDED_BY_AGREEMENT', waived: [PAY] },
    ],
  })
  const summary = summarizeRecord(record)
  expect(summary.endedBy).toEqual({ proposer: 'B', accepter: 'A' })
  expect(summary.items[1].outcome).toBe('WAIVED_BY_ENDING')

  const text = say(record)
  expect(text.standing).toEqual([
    `It ended by agreement on ${day('2026-10-20T18:00:00Z')}. Whatever was still outstanding was waived, for both of them.`,
    'Ben Ortiz proposed ending it and Ana Ruiz agreed.',
  ])
  expect(text.sides[1].items[0].outcome).toBe('Waived when the two of them agreed to end it.')
})

test('an amendment in force: the latest agreement, without what it removed', () => {
  const record = recordOf({
    state: 'CLOSED',
    outcome: 'COMPLETED',
    revisions: [
      version(V1, 1, { ...inForceStanding, status: 'REPLACED' }, ['A', 'B']),
      version(V2, 2, { ...inForceStanding, in_force_at: '2026-10-08T10:00:00Z' }, ['B', 'A'], [
        { id: BIKE, from: 'A', type: 'ITEM', description: 'A blue bicycle' },
        { id: HELMET, from: 'A', type: 'ITEM', description: 'A helmet' },
      ]),
    ],
    inForce: V2,
    statuses: [
      [BIKE, 'ACCEPTED'],
      [HELMET, 'ACCEPTED'],
      [PAY, 'REMOVED'],
    ],
  })
  const summary = summarizeRecord(record)
  expect(summary.basis).toEqual({ kind: 'AGREEMENT', sequence: 2 })
  expect(summary.items.map((item) => item.id)).toEqual([BIKE, HELMET])
  const text = say(record)
  expect(text.sides[1].nothing).toBe(en.wording.terms.nothing)
  expect(text.standing[0]).toContain('It was completed on')
})

test('nothing sent: no sides, and says so', () => {
  const record = recordOf({ state: 'CLOSED', outcome: 'NOT_AGREED', reason: 'DISCARDED', revisions: [] })
  const text = say(record)
  expect(text.basis).toBe('Nothing was ever sent.')
  expect(text.sides).toEqual([])
  expect(text.signed).toEqual([])
})

test('every sentence is filled in, in every language, for every outcome', () => {
  const shapes: Shape[] = [
    { state: 'NEGOTIATING', revisions: [version(V1, 1, { status: 'OPEN', since: 'x' }, ['A'])] },
    ...(['COMPLETED', 'ENDED_BY_AGREEMENT', 'UNRESOLVED', 'NOT_AGREED'] as const).map((outcome) => ({
      state: 'CLOSED' as const,
      outcome,
      reason: outcome === 'UNRESOLVED' ? 'INACTIVE' : outcome === 'NOT_AGREED' ? 'EXPIRED' : undefined,
      revisions: [version(V1, 1, inForceStanding, ['A', 'B'])],
      inForce: outcome === 'NOT_AGREED' ? undefined : V1,
      statuses: [
        [BIKE, 'DISPUTED'],
        [PAY, 'CLAIMED'],
      ] as [string, Status][],
      events: [{ type: 'END_PROPOSED' as const, actor: 'A' as const }],
    })),
  ]
  for (const i18n of [en, es]) {
    for (const shape of shapes) {
      const text = say(recordOf(shape), i18n)
      const all = [
        text.between,
        text.basis,
        ...text.signed,
        ...text.standing,
        ...text.sides.flatMap((side) => [
          side.heading,
          ...side.items.flatMap((item) => [...item.details, item.outcome ?? '']),
        ]),
      ]
      for (const sentence of all) expect(sentence, `${i18n.language}`).not.toMatch(/[{}]/)
    }
  }
})

describe('a series of payments', () => {
  const P1 = '00000000-0000-4000-8000-000000000021'
  const P2 = '00000000-0000-4000-8000-000000000022'
  const P3 = '00000000-0000-4000-8000-000000000023'
  const payments = [P1, P2, P3].map((id, index) => ({
    id,
    from: 'B' as const,
    type: 'MONEY' as const,
    description: `Repayment ${index + 1} of 3`,
    amount_minor: index === 2 ? 3334 : 3333,
  }))
  const series = (statuses: Status[], events: Shape['events'] = []) =>
    recordOf({
      state: 'ACTIVE',
      revisions: [version(V1, 1, inForceStanding, ['A', 'B'], payments)],
      inForce: V1,
      statuses: [P1, P2, P3].map((id, index) => [id, statuses[index]] as [string, Status]),
      events,
    })

  test('counts them under the payer, in words', () => {
    const text = say(series(['ACCEPTED', 'ACCEPTED', 'CLAIMED']))
    expect(text.sides[1].counts).toEqual(['Ben Ortiz: 2 of 3 payments confirmed'])
    expect(text.sides[0].counts).toEqual([])
    // The items are still listed under it as before, each with its own amount.
    expect(text.sides[1].items.map((item) => item.details)).toEqual([
      ['Amount: $33.33'],
      ['Amount: $33.33'],
      ['Amount: $33.34'],
    ])
  })

  test('says how many are disputed, and in Spanish too', () => {
    const record = series(['ACCEPTED', 'DISPUTED', 'PENDING'], [
      { type: 'CONTRIBUTION_DISPUTED', actor: 'A', contribution: { id: P2, description: '' } },
    ])
    expect(say(record).sides[1].counts).toEqual([
      'Ben Ortiz: 1 of 3 payments confirmed, 1 disputed',
    ])
    expect(say(record, es).sides[1].counts).toEqual([
      'Ben Ortiz: 1 de 3 pagos confirmados, 1 en disputa',
    ])
  })

  test('keeps a single payment’s wording', () => {
    const record = recordOf({
      state: 'ACTIVE',
      revisions: [version(V1, 1, inForceStanding, ['A', 'B'])],
      inForce: V1,
      statuses: [
        [BIKE, 'PENDING'],
        [PAY, 'ACCEPTED'],
      ],
    })
    expect(say(record).sides.map((side) => side.counts)).toEqual([[], []])
  })

  test('counts nothing for terms that were never agreed', () => {
    const record = recordOf({
      state: 'NEGOTIATING',
      revisions: [version(V1, 1, { status: 'OPEN', since: '2026-10-02T15:00:00Z' }, ['A'], payments)],
    })
    expect(say(record).sides.map((side) => side.counts)).toEqual([[], []])
  })
})
