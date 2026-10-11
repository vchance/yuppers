import type { components, ExchangeView } from '@yuppers/api-client'

import type { I18n } from './i18n'

type Schemas = components['schemas']
type Slot = Schemas['Slot']
type Status = Schemas['Status']
type ContributionType = Schemas['ContributionType']

/*
 * Counts for a series of payments (instalments, DESIGN.md §7.1) or of stages
 * (DESIGN.md §7.2): "2 of 3 payments confirmed". A series is nothing in the
 * model, only two or more money items from one party, or two or more service
 * or task items from one party, so it is counted from the items' own
 * statuses. Always words and numbers, never an amount, a bar or a percentage.
 */

export type SeriesKind = 'payments' | 'stages'

export interface SeriesItem {
  from: Slot
  type: ContributionType
  status: Status
}

export interface Series {
  kind: SeriesKind
  /** Who owes the payments, or provides the stages. */
  from: Slot
  total: number
  confirmed: number
  disputed: number
}

/** Which kind of series an item belongs to, if it could. */
function kindOf(type: ContributionType): SeriesKind | null {
  if (type === 'MONEY') return 'payments'
  if (type === 'SERVICE' || type === 'TASK') return 'stages'
  return null
}

/**
 * The series among some items: for each party and kind, when that party owes
 * or provides two or more. Removed items are not counted. Payments come
 * before stages, party A before party B.
 */
export function seriesOf(items: readonly SeriesItem[]): Series[] {
  const found: Series[] = []
  for (const kind of ['payments', 'stages'] as const) {
    for (const from of ['A', 'B'] as const) {
      const mine = items.filter(
        (item) => item.from === from && item.status !== 'REMOVED' && kindOf(item.type) === kind,
      )
      if (mine.length < 2) continue
      found.push({
        kind,
        from,
        total: mine.length,
        confirmed: mine.filter((item) => item.status === 'ACCEPTED').length,
        disputed: mine.filter((item) => item.status === 'DISPUTED').length,
      })
    }
  }
  return found
}

/** The series in an exchange's agreement in force, from where each item stands now. */
export function seriesInExchange(exchange: ExchangeView): Series[] {
  const inForce = exchange.in_force_revision
  if (!inForce) return []
  const statuses = new Map(exchange.contributions.map((item) => [item.id, item.status]))
  return seriesOf(
    inForce.terms.contributions.map((item) => ({
      from: item.from,
      type: item.type,
      status: statuses.get(item.id) ?? 'PENDING',
    })),
  )
}

/** A series as a sentence naming the party: "Sam: 2 of 3 payments confirmed". */
export function seriesLine(
  series: Series,
  name: string,
  i18n: Pick<I18n, 'wording' | 'fmt'>,
): string {
  const w = i18n.wording.series
  const values = {
    name,
    confirmed: series.confirmed,
    total: series.total,
    disputed: series.disputed,
  }
  const words =
    series.kind === 'payments'
      ? series.disputed > 0
        ? w.paymentsDisputed
        : w.payments
      : series.disputed > 0
        ? w.stagesDisputed
        : w.stages
  return i18n.fmt(words, values)
}

/** The counts the list carries for an exchange (`ExchangeSummary`). */
export interface SeriesCounts {
  total: number
  confirmed: number
  disputed: number
}

/**
 * Chips for the home list, beside the state: "2 of 3 payments confirmed",
 * and "1 disputed" as a chip of its own. Never an amount.
 */
export function seriesChips(
  summary: { payments?: SeriesCounts | null; stages?: SeriesCounts | null },
  i18n: Pick<I18n, 'wording' | 'fmt'>,
): string[] {
  const w = i18n.wording.series
  const chips: string[] = []
  for (const [counts, words] of [
    [summary.payments, w.chipPayments],
    [summary.stages, w.chipStages],
  ] as const) {
    if (!counts) continue
    chips.push(i18n.fmt(words, { confirmed: counts.confirmed, total: counts.total }))
    if (counts.disputed > 0) chips.push(i18n.fmt(w.chipDisputed, { disputed: counts.disputed }))
  }
  return chips
}
