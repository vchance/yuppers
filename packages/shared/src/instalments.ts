import { fromMinorUnits, toMinorUnits } from './decimal'
import type { DraftContribution } from './draft'

/*
 * Splitting a money item into instalments (DESIGN.md §7.1) and a service or
 * task item into stages (DESIGN.md §7.2), in the composer.
 *
 * Nothing here is new to the model. A split produces ordinary items, each
 * with its own ID, status, claim, confirmation and dispute; the record does
 * not know that three items are "a series". The only thing remembered is how
 * to put them back (`SplitGroup`), while nothing has been sent.
 *
 * No rate, fee, total or balance is computed, shown or kept: a payment is
 * a plain amount. The one calculation is dividing an amount the author typed
 * into parts, in minor units, with any remainder on the last part.
 */

/** The most payments or stages one split makes. A placeholder from DESIGN.md §7.1. */
export const MAX_SPLIT = 12
/** The most contributions a revision may hold (DESIGN.md §9). */
export const MAX_ITEMS = 50

/** How many parts a split may make when the working copy already holds `existing` items, one of which is being split. */
export function splitRoom(existing: number, perPart: 1 | 2 = 1): number {
  const left = MAX_ITEMS - (existing - 1)
  return Math.max(0, Math.min(MAX_SPLIT, Math.floor((left + (perPart - 1)) / perPart)))
}

// ---- Dates ---------------------------------------------------------------------

const DATE = /^(\d{4})-(\d{2})-(\d{2})$/

function parts(date: string): [number, number, number] | null {
  const match = DATE.exec(date)
  if (!match) return null
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])]
  const check = new Date(Date.UTC(year, month - 1, day))
  if (
    check.getUTCFullYear() !== year ||
    check.getUTCMonth() !== month - 1 ||
    check.getUTCDate() !== day
  ) {
    return null
  }
  return [year, month, day]
}

function written(year: number, month: number, day: number): string {
  const pad = (value: number, width: number) => String(value).padStart(width, '0')
  return `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`
}

/** Whether `date` is a real calendar date written `YYYY-MM-DD`. */
export function isDate(date: string): boolean {
  return parts(date) !== null
}

/** `date` plus a number of days. */
export function addDays(date: string, days: number): string {
  const found = parts(date)
  if (!found) return date
  const next = new Date(Date.UTC(found[0], found[1] - 1, found[2] + days))
  return written(next.getUTCFullYear(), next.getUTCMonth() + 1, next.getUTCDate())
}

/**
 * `date` plus a number of months. Where that month has no such day, the date
 * falls back to the last day of that month (DESIGN.md §7.1). Always counted
 * from the first date, so a series that starts on the 31st returns to the 31st
 * in a month that has one.
 */
export function addMonths(date: string, months: number): string {
  const found = parts(date)
  if (!found) return date
  const [year, month, day] = found
  const index = year * 12 + (month - 1) + months
  const targetYear = Math.floor(index / 12)
  const targetMonth = index - targetYear * 12
  const lastDay = new Date(Date.UTC(targetYear, targetMonth + 1, 0)).getUTCDate()
  return written(targetYear, targetMonth + 1, Math.min(day, lastDay))
}

export type Every =
  | { kind: 'WEEK' }
  | { kind: 'TWO_WEEKS' }
  | { kind: 'MONTH' }
  | { kind: 'DAYS'; days: number }

/** Every due date of a series, the first one included. */
export function scheduleDates(first: string, every: Every, count: number): string[] {
  return Array.from({ length: count }, (_, index) => {
    switch (every.kind) {
      case 'WEEK':
        return addDays(first, 7 * index)
      case 'TWO_WEEKS':
        return addDays(first, 14 * index)
      case 'MONTH':
        return addMonths(first, index)
      case 'DAYS':
        return addDays(first, every.days * index)
    }
  })
}

// ---- Amounts -------------------------------------------------------------------

/**
 * An amount divided into `count` parts, in minor units. Every part is the
 * same but the last, which also takes whatever does not divide: $100 in
 * three reads $33.33, $33.33, $33.34. `null` when a part would be nothing.
 */
export function shareOut(amountMinor: number, count: number): number[] | null {
  if (!Number.isSafeInteger(amountMinor) || !Number.isInteger(count) || count < 1) return null
  const each = Math.floor(amountMinor / count)
  if (each < 1) return null
  const parts = Array.from({ length: count }, () => each)
  parts[count - 1] += amountMinor - each * count
  return parts
}

/** How the amounts are worked out: one amount shared out, or one amount for each. */
export interface AmountChoice {
  mode: 'SHARE' | 'EACH'
  /** A plain decimal in major units, as typed; `null` if what was typed is not a number. */
  amount: string | null
}

export type AmountProblem = 'AMOUNT' | 'TOO_SMALL'

/** The amount of each part, in minor units, or what is wrong with what was typed. */
export function amountsFor(
  choice: AmountChoice,
  count: number,
  fractionDigits: number,
): { ok: true; amounts: number[] } | { ok: false; problem: AmountProblem } {
  if (choice.amount === null || choice.amount === '') return { ok: false, problem: 'AMOUNT' }
  const minor = toMinorUnits(choice.amount, fractionDigits)
  if (minor === null || minor < 1) return { ok: false, problem: 'AMOUNT' }
  if (choice.mode === 'EACH') {
    return { ok: true, amounts: Array.from({ length: count }, () => minor) }
  }
  const amounts = shareOut(minor, count)
  return amounts ? { ok: true, amounts } : { ok: false, problem: 'TOO_SMALL' }
}

// ---- The instalments sheet --------------------------------------------------------

export interface InstalmentsInput {
  count: number
  amounts: AmountChoice
  /** The first due date, `YYYY-MM-DD`. */
  first: string
  every: Every
}

export type InstalmentsProblem = 'COUNT' | 'AMOUNT' | 'TOO_SMALL' | 'DATE' | 'DAYS'

export interface InstalmentRow {
  amountMinor: number
  date: string
}

/**
 * What the sheet will add, every amount and every date, or what is wrong. The
 * sheet shows this before anything is added. `room` is how many payments the
 * revision has space for (`splitRoom`).
 */
export function planInstalments(
  input: InstalmentsInput,
  fractionDigits: number,
  room: number = MAX_SPLIT,
): { ok: true; rows: InstalmentRow[] } | { ok: false; problems: InstalmentsProblem[] } {
  const problems: InstalmentsProblem[] = []
  const countOk = Number.isInteger(input.count) && input.count >= 2 && input.count <= room
  if (!countOk) problems.push('COUNT')
  if (!isDate(input.first)) problems.push('DATE')
  if (input.every.kind === 'DAYS' && !(Number.isInteger(input.every.days) && input.every.days >= 1)) {
    problems.push('DAYS')
  }
  let amounts: number[] = []
  if (countOk) {
    const found = amountsFor(input.amounts, input.count, fractionDigits)
    if (found.ok) amounts = found.amounts
    else problems.push(found.problem)
  } else if (input.amounts.amount === null || input.amounts.amount === '') {
    problems.push('AMOUNT')
  }
  if (problems.length > 0) return { ok: false, problems }
  const dates = scheduleDates(input.first, input.every, input.count)
  return { ok: true, rows: amounts.map((amountMinor, index) => ({ amountMinor, date: dates[index] })) }
}

// ---- Splitting and putting back ---------------------------------------------------------

/**
 * What a split remembers so that it can be undone while nothing has been
 * sent: the item as it was, and the items it became. It is kept with the
 * working copy and never sent.
 */
export interface SplitGroup {
  kind: 'INSTALMENTS' | 'STAGES'
  original: DraftContribution
  /** The items the split made, in order: the first keeps the original's ID. */
  ids: string[]
}

/**
 * The item `original` as instalments: the first keeps its ID, the rest are
 * new. In an amendment that makes the first a change and the rest additions,
 * which is what splitting an item not yet confirmed is (DESIGN.md §7.1).
 *
 * `describe` writes each description in the author's language, once, from
 * the item's own description ("Repayment 1 of 3"); from then on it is the
 * author's wording like any other. Each is required if the original was, and
 * is a plain money item: no quantity, criteria, or anything but its amount
 * and due date.
 */
export function splitIntoInstalments(
  original: DraftContribution,
  rows: readonly InstalmentRow[],
  describe: (number: number, count: number) => string,
  newId: () => string,
  fractionDigits: number,
): { items: DraftContribution[]; group: SplitGroup } {
  const items = rows.map((row, index): DraftContribution => ({
    id: index === 0 ? original.id : newId(),
    from: original.from,
    type: 'MONEY',
    description: describe(index + 1, rows.length),
    quantity: '',
    unit: '',
    due: { kind: 'DATE', date: row.date },
    criteria: '',
    required: original.required,
    amount: fromMinorUnits(row.amountMinor, fractionDigits),
  }))
  return { items, group: { kind: 'INSTALMENTS', original, ids: items.map((item) => item.id) } }
}

/** Replaces the item `at` in `items` with `replacement`. */
function replaceAt<T>(items: readonly T[], at: number, replacement: readonly T[]): T[] {
  return [...items.slice(0, at), ...replacement, ...items.slice(at + 1)]
}

/**
 * `items` with the one named by `id` replaced by `replacement`, which takes
 * its place. An item that waited on the one replaced waits on the last of
 * the parts of the same kind and provider instead (the last instalment, the
 * last stage): the first part inherits the original's ID, and what waited on
 * the whole job did not mean to wait on its first piece only.
 */
export function replaceItem(
  items: readonly DraftContribution[],
  id: string,
  replacement: readonly DraftContribution[],
): DraftContribution[] {
  const at = items.findIndex((item) => item.id === id)
  if (at < 0) return [...items]
  const original = items[at]
  const last = [...replacement]
    .reverse()
    .find((part) => part.type === original.type && part.from === original.from)
  const inside = new Set(replacement.map((part) => part.id))
  const repointed = items.map((item) =>
    last &&
    !inside.has(item.id) &&
    item.due.kind === 'AFTER_CONTRIBUTION' &&
    item.due.contribution === id
      ? { ...item, due: { kind: 'AFTER_CONTRIBUTION' as const, contribution: last.id } }
      : item,
  )
  return replaceAt(repointed, at, replacement)
}

/**
 * Puts instalments back as one payment: the amounts added up, the first
 * date kept, the original's description. An instalment the author has removed
 * since is simply not counted. Returns `null` when none of them is left.
 */
export function mergeInstalments(
  items: readonly DraftContribution[],
  group: SplitGroup,
  fractionDigits: number,
): DraftContribution[] | null {
  const mine = items.filter((item) => group.ids.includes(item.id))
  if (mine.length === 0) return null
  let sum = 0
  for (const item of mine) {
    const minor = item.amount ? toMinorUnits(item.amount, fractionDigits) : 0
    sum += minor ?? 0
  }
  const first = mine[0]
  const merged: DraftContribution = {
    ...group.original,
    id: first.id,
    amount: fromMinorUnits(sum, fractionDigits),
    due: first.due,
  }
  return items.flatMap((item): DraftContribution[] => {
    if (item.id === first.id) return [merged]
    if (group.ids.includes(item.id)) return []
    // Something that waited on a part now waits on the whole.
    if (item.due.kind === 'AFTER_CONTRIBUTION' && group.ids.includes(item.due.contribution)) {
      return [{ ...item, due: { kind: 'AFTER_CONTRIBUTION', contribution: first.id } }]
    }
    return [item]
  })
}

/** The groups whose items are all still there in some form: `splits` with the dead ones dropped. */
export function liveGroups(
  items: readonly DraftContribution[],
  groups: readonly SplitGroup[],
): SplitGroup[] {
  return groups.filter((group) => group.ids.some((id) => items.some((item) => item.id === id)))
}
