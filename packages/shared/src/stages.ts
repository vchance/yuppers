import type { components } from '@yuppers/api-client'

import { fromMinorUnits } from './decimal'
import type { DraftContribution } from './draft'
import {
  amountsFor,
  isDate,
  MAX_SPLIT,
  type AmountChoice,
  type AmountProblem,
  type SplitGroup,
} from './instalments'

type Slot = components['schemas']['Slot']

/*
 * Splitting a service or task item into stages (DESIGN.md §7.2).
 *
 * Each stage is an ordinary item from the same provider: delivered by them,
 * confirmed by the other person, overdue on its own date. Stages are not
 * chained unless the author ticks the box, and a payment per stage is added
 * only if they tick that one. Nothing here turns a stage into a percentage
 * of anything: how far along a job is, is how many stages are confirmed.
 */

export interface StagesInput {
  count: number
  /** One name per stage, as typed. */
  names: readonly string[]
  /** One due date per stage, `YYYY-MM-DD`, or empty for "when the agreement is signed". */
  dates: readonly string[]
  /** Each stage after the first is due once the one before it is confirmed. */
  chain: boolean
  /** A payment from the other person for each stage, or `null` for none. */
  pay: AmountChoice | null
}

export type StagesProblem = 'COUNT' | 'NAME' | 'DATE' | AmountProblem

export interface StagesPlan {
  stages: { name: string; date: string }[]
  /** Minor units for each stage's payment, when there are payments. */
  payments: number[] | null
}

/** What the sheet will add, or what is wrong. `room` is how many stages the revision has space for. */
export function planStages(
  input: StagesInput,
  fractionDigits: number,
  room: number = MAX_SPLIT,
): { ok: true; plan: StagesPlan } | { ok: false; problems: StagesProblem[] } {
  const problems: StagesProblem[] = []
  const countOk = Number.isInteger(input.count) && input.count >= 2 && input.count <= room
  if (!countOk) problems.push('COUNT')
  const names = Array.from({ length: input.count }, (_, index) => (input.names[index] ?? '').trim())
  if (countOk && names.some((name) => name === '')) problems.push('NAME')
  const dates = Array.from({ length: input.count }, (_, index) => input.dates[index] ?? '')
  // With the chain, the dates after the first stage are not used.
  const used = input.chain ? dates.slice(0, 1) : dates
  if (countOk && used.some((date) => date !== '' && !isDate(date))) problems.push('DATE')

  let payments: number[] | null = null
  if (input.pay && countOk) {
    const found = amountsFor(input.pay, input.count, fractionDigits)
    if (found.ok) payments = found.amounts
    else problems.push(found.problem)
  }
  if (problems.length > 0) return { ok: false, problems }
  return {
    ok: true,
    plan: {
      stages: names.map((name, index) => ({ name, date: dates[index] })),
      payments,
    },
  }
}

/**
 * The item `original` as stages: the first keeps its ID, the rest are new.
 * With payments, each stage is followed by its payment, which is due once
 * that stage is confirmed: a payment waits on one item only, so it waits on
 * its own stage. The stages come first so that the pairs read stage, then
 * its payment.
 *
 * `payment` writes a payment's description from its stage's name, in the
 * author's language, once.
 */
export function splitIntoStages(
  original: DraftContribution,
  plan: StagesPlan,
  options: { chain: boolean; payer: Slot },
  payment: (stage: string) => string,
  newId: () => string,
  fractionDigits: number,
): { items: DraftContribution[]; group: SplitGroup } {
  const items: DraftContribution[] = []
  const ids = plan.stages.map((_, index) => (index === 0 ? original.id : newId()))
  plan.stages.forEach((stage, index) => {
    const chained = options.chain && index > 0
    items.push({
      id: ids[index],
      from: original.from,
      type: original.type,
      description: stage.name,
      quantity: '',
      unit: '',
      due: chained
        ? { kind: 'AFTER_CONTRIBUTION', contribution: ids[index - 1] }
        : stage.date === ''
          ? { kind: 'ON_AGREEMENT' }
          : { kind: 'DATE', date: stage.date },
      criteria: '',
      required: original.required,
      amount: '',
    })
    if (plan.payments) {
      items.push({
        id: newId(),
        from: options.payer,
        type: 'MONEY',
        description: payment(stage.name),
        quantity: '',
        unit: '',
        due: { kind: 'AFTER_CONTRIBUTION', contribution: ids[index] },
        criteria: '',
        required: original.required,
        amount: fromMinorUnits(plan.payments[index], fractionDigits),
      })
    }
  })
  return { items, group: { kind: 'STAGES', original, ids: items.map((item) => item.id) } }
}

/**
 * Puts stages back as one job: the original item, in the place of the first
 * stage. Any payment that was added with them goes too, as does anything that
 * waited on a stage; an item that waited on one is pointed at the original.
 */
export function mergeStages(
  items: readonly DraftContribution[],
  group: SplitGroup,
): DraftContribution[] | null {
  const mine = items.filter((item) => group.ids.includes(item.id))
  if (mine.length === 0) return null
  const first = items.find((item) => item.id === group.ids[0]) ?? mine[0]
  const merged: DraftContribution = { ...group.original, id: first.id }
  return items.flatMap((item): DraftContribution[] => {
    if (item.id === first.id) return [merged]
    if (group.ids.includes(item.id)) return []
    // Something else that waited on one of the stages waits on the job now.
    if (item.due.kind === 'AFTER_CONTRIBUTION' && group.ids.includes(item.due.contribution)) {
      return [{ ...item, due: { kind: 'AFTER_CONTRIBUTION', contribution: first.id } }]
    }
    return [item]
  })
}
