import { describe, expect, it } from 'vitest'

import { newContribution, readDraft, type Draft, type DraftContribution } from './draft'
import {
  addDays,
  addMonths,
  amountsFor,
  liveGroups,
  mergeInstalments,
  planInstalments,
  replaceItem,
  scheduleDates,
  shareOut,
  splitIntoInstalments,
  splitRoom,
} from './instalments'
import { swapSides } from './templates'

function repayment(over: Partial<DraftContribution> = {}): DraftContribution {
  return {
    ...newContribution('original', 'B'),
    type: 'MONEY',
    description: 'Repayment',
    amount: '100.00',
    due: { kind: 'DATE', date: '2026-11-01' },
    ...over,
  }
}

let counter = 0
const newId = () => `new-${(counter += 1)}`
const describe3 = (number: number, count: number) => `Repayment ${number} of ${count}`

describe('dividing an amount', () => {
  it('puts the remainder on the last part, in minor units', () => {
    expect(shareOut(10_000, 3)).toEqual([3333, 3333, 3334])
    expect(shareOut(10_000, 4)).toEqual([2500, 2500, 2500, 2500])
    expect(shareOut(5, 2)).toEqual([2, 3])
  })

  it('refuses a part that would be nothing', () => {
    expect(shareOut(2, 3)).toBeNull()
    expect(shareOut(0, 2)).toBeNull()
    expect(shareOut(100, 0)).toBeNull()
  })

  it('works out "the same amount for each" as typed', () => {
    expect(amountsFor({ mode: 'EACH', amount: '12.50' }, 3, 2)).toEqual({
      ok: true,
      amounts: [1250, 1250, 1250],
    })
    expect(amountsFor({ mode: 'SHARE', amount: '100' }, 3, 2)).toEqual({
      ok: true,
      amounts: [3333, 3333, 3334],
    })
  })

  it('says what is wrong with an amount', () => {
    expect(amountsFor({ mode: 'SHARE', amount: '' }, 3, 2)).toEqual({ ok: false, problem: 'AMOUNT' })
    expect(amountsFor({ mode: 'SHARE', amount: null }, 3, 2)).toEqual({ ok: false, problem: 'AMOUNT' })
    expect(amountsFor({ mode: 'SHARE', amount: '0.02' }, 3, 2)).toEqual({
      ok: false,
      problem: 'TOO_SMALL',
    })
  })
})

describe('due dates', () => {
  it('adds days and weeks', () => {
    expect(addDays('2026-10-30', 3)).toBe('2026-11-02')
    expect(scheduleDates('2026-11-01', { kind: 'WEEK' }, 3)).toEqual([
      '2026-11-01',
      '2026-11-08',
      '2026-11-15',
    ])
    expect(scheduleDates('2026-11-01', { kind: 'TWO_WEEKS' }, 3)).toEqual([
      '2026-11-01',
      '2026-11-15',
      '2026-11-29',
    ])
    expect(scheduleDates('2026-11-01', { kind: 'DAYS', days: 10 }, 3)).toEqual([
      '2026-11-01',
      '2026-11-11',
      '2026-11-21',
    ])
  })

  it('falls back to the last day of a month that has no such day, counting from the first date', () => {
    expect(addMonths('2026-01-31', 1)).toBe('2026-02-28')
    expect(addMonths('2028-01-31', 1)).toBe('2028-02-29')
    expect(scheduleDates('2026-01-31', { kind: 'MONTH' }, 4)).toEqual([
      '2026-01-31',
      '2026-02-28',
      '2026-03-31',
      '2026-04-30',
    ])
    expect(addMonths('2026-11-15', 2)).toBe('2027-01-15')
    expect(addMonths('2026-12-31', 12)).toBe('2027-12-31')
  })
})

describe('the instalments sheet', () => {
  const input = {
    count: 3,
    amounts: { mode: 'SHARE' as const, amount: '100' },
    first: '2026-11-01',
    every: { kind: 'MONTH' as const },
  }

  it('shows every amount and every date before anything is added', () => {
    expect(planInstalments(input, 2)).toEqual({
      ok: true,
      rows: [
        { amountMinor: 3333, date: '2026-11-01' },
        { amountMinor: 3333, date: '2026-12-01' },
        { amountMinor: 3334, date: '2027-01-01' },
      ],
    })
  })

  it('keeps to 2 and the most the revision has room for', () => {
    expect(planInstalments({ ...input, count: 1 }, 2)).toMatchObject({ ok: false, problems: ['COUNT'] })
    expect(planInstalments({ ...input, count: 13 }, 2)).toMatchObject({ ok: false, problems: ['COUNT'] })
    expect(planInstalments({ ...input, count: 5 }, 2, 4)).toMatchObject({ ok: false, problems: ['COUNT'] })
    expect(planInstalments({ ...input, count: 12 }, 2)).toMatchObject({ ok: true })
  })

  it('names every problem at once', () => {
    expect(
      planInstalments(
        { count: 3, amounts: { mode: 'EACH', amount: '' }, first: '', every: { kind: 'DAYS', days: 0 } },
        2,
      ),
    ).toEqual({ ok: false, problems: ['DATE', 'DAYS', 'AMOUNT'] })
  })

  it('has room for the revision’s bound of 50 items', () => {
    expect(splitRoom(1)).toBe(12)
    expect(splitRoom(40)).toBe(11)
    expect(splitRoom(50)).toBe(1)
    expect(splitRoom(50, 2)).toBe(1)
    expect(splitRoom(45, 2)).toBe(3)
  })
})

describe('splitting into instalments', () => {
  const rows = [
    { amountMinor: 3333, date: '2026-11-01' },
    { amountMinor: 3333, date: '2026-12-01' },
    { amountMinor: 3334, date: '2027-01-01' },
  ]

  it('makes plain money items from the same payer, in order, each required if the original was', () => {
    const original = repayment({ criteria: 'x', quantity: '2', required: false })
    const { items } = splitIntoInstalments(original, rows, describe3, newId, 2)
    expect(items.map((item) => item.description)).toEqual([
      'Repayment 1 of 3',
      'Repayment 2 of 3',
      'Repayment 3 of 3',
    ])
    expect(items.map((item) => item.amount)).toEqual(['33.33', '33.33', '33.34'])
    expect(items.map((item) => item.due)).toEqual(rows.map((row) => ({ kind: 'DATE', date: row.date })))
    for (const item of items) {
      expect(item).toMatchObject({
        from: 'B',
        type: 'MONEY',
        required: false,
        criteria: '',
        quantity: '',
        unit: '',
      })
    }
    // No field beyond the item's own: nothing for a rate, a fee or a balance.
    expect(Object.keys(items[0]).sort()).toEqual(Object.keys(newContribution('x', 'A')).sort())
  })

  it('keeps the original’s ID on the first, so an amendment reads change and add', () => {
    const { items, group } = splitIntoInstalments(repayment(), rows, describe3, newId, 2)
    expect(items[0].id).toBe('original')
    expect(new Set(items.map((item) => item.id)).size).toBe(3)
    expect(group.ids).toEqual(items.map((item) => item.id))
  })

  it('takes the place of the item it split', () => {
    const before = newContribution('before', 'A')
    const after = newContribution('after', 'A')
    const { items } = splitIntoInstalments(repayment(), rows, describe3, newId, 2)
    expect(replaceItem([before, repayment(), after], 'original', items).map((item) => item.id)).toEqual([
      'before',
      ...items.map((item) => item.id),
      'after',
    ])
  })

  it('puts them back as one payment: amounts summed, first date kept, the original’s words', () => {
    const original = repayment()
    const { items, group } = splitIntoInstalments(original, rows, describe3, newId, 2)
    const others = [newContribution('before', 'A'), newContribution('after', 'A')]
    const list = replaceItem([others[0], original, others[1]], 'original', items)
    const merged = mergeInstalments(list, group, 2)
    expect(merged?.map((item) => item.id)).toEqual(['before', 'original', 'after'])
    expect(merged?.[1]).toMatchObject({
      description: 'Repayment',
      amount: '100.00',
      due: { kind: 'DATE', date: '2026-11-01' },
      type: 'MONEY',
      from: 'B',
    })
  })

  it('puts back what is left when one was removed or reworded', () => {
    const { items, group } = splitIntoInstalments(repayment(), rows, describe3, newId, 2)
    const edited = items.slice(0, 2).map((item, index) =>
      index === 0 ? { ...item, due: { kind: 'DATE' as const, date: '2026-11-05' } } : item,
    )
    const merged = mergeInstalments(edited, group, 2)
    expect(merged).toHaveLength(1)
    expect(merged?.[0]).toMatchObject({ amount: '66.66', due: { kind: 'DATE', date: '2026-11-05' } })
    expect(mergeInstalments([newContribution('other', 'A')], group, 2)).toBeNull()
  })

  it('forgets a split once none of its items is left', () => {
    const { items, group } = splitIntoInstalments(repayment(), rows, describe3, newId, 2)
    expect(liveGroups(items, [group])).toEqual([group])
    expect(liveGroups([newContribution('other', 'A')], [group])).toEqual([])
  })
})

describe('what is kept with the working copy', () => {
  it('reads a stored split back, and drops one it cannot read', () => {
    const { items, group } = splitIntoInstalments(
      repayment(),
      [
        { amountMinor: 5000, date: '2026-11-01' },
        { amountMinor: 5000, date: '2026-12-01' },
      ],
      describe3,
      newId,
      2,
    )
    const draft: Draft = {
      format: 1,
      base: null,
      partyA: 'Ana',
      partyB: 'Ben',
      terms: '',
      note: '',
      contributions: items,
      splits: [group],
    }
    const stored = JSON.parse(JSON.stringify(draft)) as unknown
    expect(readDraft(stored)?.splits).toEqual([group])

    const broken = JSON.parse(JSON.stringify(draft)) as { splits: unknown[] }
    broken.splits.push({ kind: 'NOPE', original: {}, ids: [] }, 'x')
    expect(readDraft(broken)?.splits).toEqual([group])
    expect(readDraft({ ...draft, splits: undefined })?.splits).toBeUndefined()
  })

  it('flips the original with the sides, so putting a split back keeps the side it is on', () => {
    const { items, group } = splitIntoInstalments(
      repayment(),
      [
        { amountMinor: 5000, date: '2026-11-01' },
        { amountMinor: 5000, date: '2026-12-01' },
      ],
      describe3,
      newId,
      2,
    )
    const swapped = swapSides({
      format: 1,
      base: null,
      partyA: '',
      partyB: '',
      terms: '',
      note: '',
      contributions: items,
      splits: [group],
    })
    expect(swapped.contributions[0].from).toBe('A')
    expect(swapped.splits?.[0].original.from).toBe('A')
  })
})

describe('what waits on a split item', () => {
  const rows = [
    { amountMinor: 5000, date: '2026-11-01' },
    { amountMinor: 5000, date: '2026-12-01' },
    { amountMinor: 5000, date: '2027-01-01' },
  ]
  const waiter = (): DraftContribution => ({
    ...newContribution('waiter', 'A'),
    due: { kind: 'AFTER_CONTRIBUTION', contribution: 'original' },
  })

  it('waits on the last instalment, not the first part that inherited the ID', () => {
    const { items } = splitIntoInstalments(repayment(), rows, describe3, newId, 2)
    const list = replaceItem([repayment(), waiter()], 'original', items)
    expect(list.at(-1)?.due).toEqual({ kind: 'AFTER_CONTRIBUTION', contribution: items[2].id })
    // The parts themselves are not pointed anywhere new.
    expect(list.slice(0, 3).map((item) => item.due.kind)).toEqual(['DATE', 'DATE', 'DATE'])
  })

  it('goes back to the whole when the instalments are put back', () => {
    const { items, group } = splitIntoInstalments(repayment(), rows, describe3, newId, 2)
    const list = replaceItem([repayment(), waiter()], 'original', items)
    const merged = mergeInstalments(list, group, 2)
    expect(merged?.at(-1)?.due).toEqual({ kind: 'AFTER_CONTRIBUTION', contribution: 'original' })
  })
})
