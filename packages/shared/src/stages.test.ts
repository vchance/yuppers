import { describe, expect, it } from 'vitest'

import { buildTerms, newContribution, type Draft, type DraftContribution } from './draft'
import { replaceItem } from './instalments'
import { mergeStages, planStages, splitIntoStages } from './stages'

function fence(over: Partial<DraftContribution> = {}): DraftContribution {
  return {
    ...newContribution('job', 'A'),
    type: 'SERVICE',
    description: 'Repair the fence',
    required: true,
    ...over,
  }
}

let counter = 0
const newId = () => `new-${(counter += 1)}`
const input = {
  count: 3,
  names: ['Posts set', 'Panels up', 'Painted'],
  dates: ['2026-11-01', '', '2026-11-20'],
  chain: false,
  pay: null,
}

describe('the stages sheet', () => {
  it('plans a name and a date for each stage', () => {
    expect(planStages(input, 2)).toEqual({
      ok: true,
      plan: {
        stages: [
          { name: 'Posts set', date: '2026-11-01' },
          { name: 'Panels up', date: '' },
          { name: 'Painted', date: '2026-11-20' },
        ],
        payments: null,
      },
    })
  })

  it('names what is wrong', () => {
    expect(planStages({ ...input, count: 1 }, 2)).toMatchObject({ ok: false, problems: ['COUNT'] })
    expect(planStages({ ...input, count: 13 }, 2)).toMatchObject({ ok: false, problems: ['COUNT'] })
    expect(planStages({ ...input, names: ['Posts set', ' ', 'Painted'] }, 2)).toMatchObject({
      ok: false,
      problems: ['NAME'],
    })
    expect(planStages({ ...input, dates: ['2026-02-30', '', ''] }, 2)).toMatchObject({
      ok: false,
      problems: ['DATE'],
    })
    expect(
      planStages({ ...input, pay: { mode: 'SHARE', amount: '' } }, 2),
    ).toMatchObject({ ok: false, problems: ['AMOUNT'] })
  })

  it('does not look at dates it will not use when the stages are chained', () => {
    expect(planStages({ ...input, chain: true, dates: ['2026-11-01', 'nonsense', ''] }, 2)).toMatchObject({
      ok: true,
    })
  })

  it('works out a payment for each stage the way instalments are worked out', () => {
    const planned = planStages({ ...input, pay: { mode: 'SHARE', amount: '100' } }, 2)
    expect(planned).toMatchObject({ ok: true, plan: { payments: [3333, 3333, 3334] } })
  })
})

describe('splitting into stages', () => {
  const plan = {
    stages: [
      { name: 'Posts set', date: '2026-11-01' },
      { name: 'Panels up', date: '' },
      { name: 'Painted', date: '2026-11-20' },
    ],
    payments: null,
  }

  it('makes ordinary items from the same provider, each with its own words to fill in', () => {
    const { items } = splitIntoStages(
      fence({ criteria: 'done', required: false }),
      plan,
      { chain: false, payer: 'B' },
      (stage) => `Payment for ${stage}`,
      newId,
      2,
    )
    expect(items.map((item) => item.description)).toEqual(['Posts set', 'Panels up', 'Painted'])
    expect(items[0].id).toBe('job')
    expect(items.map((item) => item.due)).toEqual([
      { kind: 'DATE', date: '2026-11-01' },
      { kind: 'ON_AGREEMENT' },
      { kind: 'DATE', date: '2026-11-20' },
    ])
    for (const item of items) {
      expect(item).toMatchObject({ from: 'A', type: 'SERVICE', required: false, criteria: '' })
    }
  })

  it('does not chain the stages unless asked to', () => {
    const { items } = splitIntoStages(
      fence(),
      plan,
      { chain: false, payer: 'B' },
      (stage) => stage,
      newId,
      2,
    )
    expect(items.some((item) => item.due.kind === 'AFTER_CONTRIBUTION')).toBe(false)
  })

  it('chains each stage to the one before when ticked, leaving the first on its date', () => {
    const { items } = splitIntoStages(
      fence(),
      plan,
      { chain: true, payer: 'B' },
      (stage) => stage,
      newId,
      2,
    )
    expect(items[0].due).toEqual({ kind: 'DATE', date: '2026-11-01' })
    expect(items[1].due).toEqual({ kind: 'AFTER_CONTRIBUTION', contribution: items[0].id })
    expect(items[2].due).toEqual({ kind: 'AFTER_CONTRIBUTION', contribution: items[1].id })
  })

  it('adds a payment from the other person for each stage, due once its stage is confirmed', () => {
    const paid = { ...plan, payments: [3333, 3333, 3334] }
    const { items } = splitIntoStages(
      fence(),
      paid,
      { chain: false, payer: 'B' },
      (stage) => `Payment for ${stage}`,
      newId,
      2,
    )
    expect(items.map((item) => item.type)).toEqual([
      'SERVICE',
      'MONEY',
      'SERVICE',
      'MONEY',
      'SERVICE',
      'MONEY',
    ])
    // Stage, then its payment: the pairs read together.
    expect(items[1]).toMatchObject({
      from: 'B',
      description: 'Payment for Posts set',
      amount: '33.33',
      due: { kind: 'AFTER_CONTRIBUTION', contribution: items[0].id },
    })
    expect(items[5]).toMatchObject({
      amount: '33.34',
      due: { kind: 'AFTER_CONTRIBUTION', contribution: items[4].id },
    })
    // Each waits on exactly one item, so the terms build without a cycle.
    const draft: Draft = {
      format: 1,
      base: null,
      partyA: 'Dana',
      partyB: 'Sam',
      terms: '',
      note: '',
      contributions: items,
    }
    expect(buildTerms(draft, 2)).toMatchObject({ ok: true })
  })

  it('also builds when chained and paid per stage', () => {
    const paid = { ...plan, payments: [3333, 3333, 3334] }
    const { items } = splitIntoStages(
      fence(),
      paid,
      { chain: true, payer: 'B' },
      (stage) => stage,
      newId,
      2,
    )
    const draft: Draft = {
      format: 1,
      base: null,
      partyA: 'Dana',
      partyB: 'Sam',
      terms: '',
      note: '',
      contributions: items,
    }
    expect(buildTerms(draft, 2)).toMatchObject({ ok: true })
  })

  it('puts the job back together, with its payments gone and anything that waited on a stage pointed at it', () => {
    const paid = { ...plan, payments: [3333, 3333, 3334] }
    const original = fence()
    const { items, group } = splitIntoStages(
      original,
      paid,
      { chain: false, payer: 'B' },
      (stage) => stage,
      newId,
      2,
    )
    const bystander = {
      ...newContribution('bystander', 'B'),
      due: { kind: 'AFTER_CONTRIBUTION' as const, contribution: items[2].id },
    }
    const list = replaceItem([bystander, original], 'job', items)
    const merged = mergeStages(list, group)
    expect(merged?.map((item) => item.id)).toEqual(['bystander', 'job'])
    expect(merged?.[1]).toEqual(original)
    expect(merged?.[0].due).toEqual({ kind: 'AFTER_CONTRIBUTION', contribution: 'job' })
  })
})
