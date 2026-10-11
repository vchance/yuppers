import type { components } from '@yuppers/api-client'

import type { I18n } from './i18n'
import { isMoney } from './money'
import { documentOf, type RecordDocument, type RecordRevision } from './record'
import { seriesLine, seriesOf, type SeriesItem } from './series'
import type { ClosedReason } from './wording/types'

type Schemas = components['schemas']
type Slot = Schemas['Slot']
type Status = Schemas['Status']

/*
 * The plain summary at the top of a record (DESIGN.md §14.1): who the two
 * parties are, what each agreed to give, who signed and when, how it ended or
 * where it stands, and what became of each item. It is worked out here, from
 * the record document alone, so that every client says the same thing; the
 * full record under it stays what counts.
 */

/** What became of one item, in the summary's words. */
export type SummaryOutcome =
  /** Accepted by the recipient. */
  | 'CONFIRMED'
  /** Waived by the recipient, on its own. */
  | 'WAIVED'
  /** Released, undelivered, when the exchange ended by agreement. */
  | 'WAIVED_BY_ENDING'
  | 'DISPUTED'
  /** Marked delivered, not confirmed. */
  | 'CLAIMED'
  /** Nothing done about it yet, and the exchange goes on. */
  | 'OUTSTANDING'
  /** Nothing done about it, and the exchange has closed. */
  | 'OUTSTANDING_CLOSED'

export interface SummaryItem {
  id: string
  /** Who gives it. */
  from: Slot
  description: string
  type: Schemas['ContributionType']
  money: boolean
  amountMinor: number | null
  quantity: Schemas['QuantityDto'] | null
  /**
   * What became of it, or `null` when nothing was ever agreed: an item
   * that was only proposed has no outcome.
   */
  outcome: SummaryOutcome | null
  /** Who did what `outcome` says: confirmed, waived, disputed or marked it delivered. */
  by: Slot | null
}

export interface SummarySignature {
  party: Slot
  name: string
  at: string
}

export interface RecordSummary {
  parties: Schemas['Parties']
  /**
   * Which version the items come from: the agreement both signed, or the
   * last version sent if they never did, or none if nothing was sent.
   */
  basis: { kind: 'AGREEMENT' | 'LAST' | 'NONE'; sequence: number | null }
  items: SummaryItem[]
  /** The signatures on that version that count, oldest first. */
  signatures: SummarySignature[]
  /** The parties who did not sign it. */
  unsigned: Slot[]
  /** When it came into force, for an agreement. */
  inForceAt: string | null
  state: Schemas['StateDto']
  outcome: Schemas['OutcomeDto'] | null
  closedAt: string | null
  closedReason: string | null
  /** For an exchange ended by agreement: who proposed ending, and who agreed. */
  endedBy: { proposer: Slot; accepter: Slot } | null
  /** A version waiting to be signed, by its number. */
  waiting: number | null
  endProposedBy: Slot | null
  closeRequest: { by: Slot; at: string } | null
}

const other = (slot: Slot): Slot => (slot === 'A' ? 'B' : 'A')

/** The version the summary reads the items from. */
function basisOf(record: RecordDocument): { revision: RecordRevision | null; agreed: boolean } {
  const { revisions, exchange } = record
  const inForceId = exchange.in_force_revision?.id
  const named = inForceId ? revisions.find((revision) => revision.id === inForceId) : undefined
  if (named) return { revision: named, agreed: true }
  // The latest version that was ever in force, should the exchange no longer name one.
  const agreed = revisions
    .filter((revision) => revision.standing.in_force_at)
    .sort((a, b) => b.sequence - a.sequence)[0]
  if (agreed) return { revision: agreed, agreed: true }
  const last = [...revisions].sort((a, b) => b.sequence - a.sequence)[0]
  return { revision: last ?? null, agreed: false }
}

/** Who last did `type` to a contribution, as the history says. */
function lastActor(record: RecordDocument, type: Schemas['EventType'], id: string): Slot | null {
  for (let i = record.events.length - 1; i >= 0; i -= 1) {
    const event = record.events[i]
    if (event.type === type && event.contribution?.id === id && event.actor !== 'SYSTEM') {
      return event.actor
    }
  }
  return null
}

function outcomeOf(
  record: RecordDocument,
  id: string,
  from: Slot,
  status: Status,
  waivedByEnding: ReadonlySet<string>,
): { outcome: SummaryOutcome | null; by: Slot | null } {
  const recipient = other(from)
  const closed = record.exchange.state === 'CLOSED'
  switch (status) {
    case 'ACCEPTED':
      return {
        outcome: 'CONFIRMED',
        by: lastActor(record, 'CONTRIBUTION_CONFIRMED', id) ?? recipient,
      }
    case 'WAIVED':
      if (waivedByEnding.has(id)) return { outcome: 'WAIVED_BY_ENDING', by: null }
      return { outcome: 'WAIVED', by: lastActor(record, 'CONTRIBUTION_WAIVED', id) ?? recipient }
    case 'DISPUTED':
      return {
        outcome: 'DISPUTED',
        by: lastActor(record, 'CONTRIBUTION_DISPUTED', id) ?? recipient,
      }
    case 'CLAIMED':
      return { outcome: 'CLAIMED', by: from }
    case 'PENDING':
      return { outcome: closed ? 'OUTSTANDING_CLOSED' : 'OUTSTANDING', by: null }
    default:
      return { outcome: null, by: null }
  }
}

/** The summary of a whole record. */
export function summarizeRecord(record: RecordDocument): RecordSummary {
  const { exchange, events } = record
  const { revision, agreed } = basisOf(record)
  const statuses = new Map(record.contributions.map((item) => [item.id, item.status]))

  const closing = [...events].reverse().find((event) => event.type === 'EXCHANGE_CLOSED')
  const waivedByEnding = new Set(
    exchange.closed_outcome === 'ENDED_BY_AGREEMENT' ? (closing?.waived ?? []) : [],
  )

  const items: SummaryItem[] = ((revision ? documentOf(revision) : undefined)?.contributions ?? []).flatMap((contribution) => {
    const status = statuses.get(contribution.id) ?? 'PENDING'
    // A contribution an amendment removed is not part of what was agreed.
    if (agreed && status === 'REMOVED') return []
    const { outcome, by } = agreed
      ? outcomeOf(record, contribution.id, contribution.from, status, waivedByEnding)
      : { outcome: null, by: null }
    return [
      {
        id: contribution.id,
        from: contribution.from,
        description: contribution.description,
        type: contribution.type,
        money: isMoney(contribution),
        amountMinor: contribution.amount_minor ?? null,
        quantity: contribution.quantity ?? null,
        outcome,
        by,
      },
    ]
  })

  const signatures = (revision?.signatures ?? [])
    .map((signature) => ({ party: signature.party, name: signature.name, at: signature.signed_at }))
    .sort((a, b) => a.at.localeCompare(b.at))
  const signed = new Set(signatures.map((signature) => signature.party))

  let endedBy: RecordSummary['endedBy'] = null
  if (exchange.closed_outcome === 'ENDED_BY_AGREEMENT') {
    const proposal = [...events].reverse().find((event) => event.type === 'END_PROPOSED')
    if (proposal && proposal.actor !== 'SYSTEM') {
      endedBy = { proposer: proposal.actor, accepter: other(proposal.actor) }
    }
  }

  return {
    parties: record.parties,
    basis: {
      kind: revision ? (agreed ? 'AGREEMENT' : 'LAST') : 'NONE',
      sequence: revision?.sequence ?? null,
    },
    items,
    signatures,
    unsigned: revision ? (['A', 'B'] as const).filter((slot) => !signed.has(slot)) : [],
    inForceAt: agreed ? (revision?.standing.in_force_at ?? null) : null,
    state: exchange.state,
    outcome: exchange.closed_outcome ?? null,
    closedAt: exchange.closed_at ?? null,
    closedReason: exchange.closed_reason ?? null,
    endedBy,
    waiting: exchange.state === 'CLOSED' ? null : (exchange.open_revision?.sequence ?? null),
    endProposedBy: exchange.state === 'ACTIVE' ? (exchange.end_proposed_by ?? null) : null,
    closeRequest:
      exchange.state === 'ACTIVE' && exchange.close_requested_by && exchange.close_requested_at
        ? { by: exchange.close_requested_by, at: exchange.close_requested_at }
        : null,
  }
}

/** One side's items, worded. */
export interface SummarySide {
  slot: Slot
  heading: string
  items: {
    id: string
    /** The parties' own words: shown as such, never translated. */
    description: string
    /** Amount or quantity, in the product's words. */
    details: string[]
    /** What became of it; `null` when nothing was agreed. */
    outcome: string | null
  }[]
  /**
   * Where a series stands, when this side owes two or more payments or
   * provides two or more stages (DESIGN.md §7.1, §7.2): "Sam: 2 of 3 payments
   * confirmed". Counts only, in words. Empty for anything else.
   */
  counts: string[]
  /** Said when this side gives nothing. */
  nothing: string | null
}

/** The summary in sentences, the same in every client. */
export interface SummaryText {
  heading: string
  intro: string
  between: string
  basis: string
  sides: SummarySide[]
  /** Who signed the version the items come from, when, and when it came into force. */
  signed: string[]
  /** Where it stands now, or how it ended. */
  standing: string[]
}

/**
 * Puts a summary into words. `day` writes a moment as a date in the
 * exchange's own time zone, since the summary may be read anywhere.
 */
export function summaryText(
  summary: RecordSummary,
  i18n: Pick<I18n, 'wording' | 'fmt' | 'money'>,
  currency: string,
  day: (instant: string) => string,
): SummaryText {
  const { wording, fmt, money } = i18n
  const w = wording.record.summary
  const name = (slot: Slot) => summary.parties[slot] || wording.party.other

  const basis =
    summary.basis.kind === 'AGREEMENT'
      ? fmt(w.basisAgreement, { number: summary.basis.sequence ?? 0 })
      : summary.basis.kind === 'LAST'
        ? fmt(w.basisLast, { number: summary.basis.sequence ?? 0 })
        : w.basisNone

  const sides: SummarySide[] =
    summary.basis.kind === 'NONE'
      ? []
      : (['A', 'B'] as const).map((slot) => {
          const items = summary.items
            .filter((item) => item.from === slot)
            .map((item) => {
              const details: string[] = []
              if (item.amountMinor != null) {
                details.push(fmt(wording.terms.amount, { amount: money(item.amountMinor, currency) }))
              }
              if (item.quantity) {
                details.push(
                  item.quantity.unit
                    ? fmt(wording.terms.quantityWithUnit, {
                        amount: item.quantity.amount,
                        unit: item.quantity.unit,
                      })
                    : fmt(wording.terms.quantity, { amount: item.quantity.amount }),
                )
              }
              const words = item.money ? w.moneyOutcome : w.outcome
              const outcome = item.outcome
                ? fmt(words[item.outcome], {
                    name: item.by ? name(item.by) : '',
                    provider: name(item.from),
                    other: name(other(item.from)),
                  })
                : null
              return { id: item.id, description: item.description, details, outcome }
            })
          // A series is counted only where something was agreed.
          const counted: SeriesItem[] =
            summary.basis.kind === 'AGREEMENT'
              ? summary.items
                  .filter((item) => item.from === slot && item.outcome !== null)
                  .map((item) => ({
                    from: item.from,
                    type: item.type,
                    status:
                      item.outcome === 'CONFIRMED'
                        ? 'ACCEPTED'
                        : item.outcome === 'DISPUTED'
                          ? 'DISPUTED'
                          : 'PENDING',
                  }))
              : []
          return {
            slot,
            heading: fmt(summary.basis.kind === 'AGREEMENT' ? w.givesAgreed : w.givesLast, {
              name: name(slot),
            }),
            items,
            counts: seriesOf(counted).map((series) => seriesLine(series, name(slot), i18n)),
            nothing: items.length === 0 ? wording.terms.nothing : null,
          }
        })

  const signed = [
    ...summary.signatures.map((signature) =>
      fmt(w.signedBy, { name: signature.name || name(signature.party), date: day(signature.at) }),
    ),
    ...summary.unsigned.map((slot) => fmt(w.notSignedBy, { name: name(slot) })),
    ...(summary.inForceAt ? [fmt(w.inForceFrom, { date: day(summary.inForceAt) })] : []),
  ]

  const standing: string[] = []
  if (summary.state === 'CLOSED') {
    const outcome = summary.outcome ?? 'UNRESOLVED'
    standing.push(fmt(w.standing[outcome], { date: summary.closedAt ? day(summary.closedAt) : '' }))
    if (outcome === 'ENDED_BY_AGREEMENT' && summary.endedBy) {
      standing.push(
        fmt(w.endedProposedBy, {
          proposer: name(summary.endedBy.proposer),
          accepter: name(summary.endedBy.accepter),
        }),
      )
    }
    const reason = summary.closedReason
    if (
      (outcome === 'UNRESOLVED' || outcome === 'NOT_AGREED') &&
      reason &&
      Object.hasOwn(wording.record.closedReasons, reason)
    ) {
      standing.push(wording.record.closedReasons[reason as ClosedReason])
    }
  } else {
    standing.push(w.standing[summary.state])
    if (summary.waiting !== null) {
      standing.push(fmt(wording.record.waiting, { number: summary.waiting }))
    }
    if (summary.endProposedBy) {
      standing.push(fmt(wording.record.endProposed, { name: name(summary.endProposedBy) }))
    }
    if (summary.closeRequest) {
      standing.push(
        fmt(wording.record.closeRequested, {
          name: name(summary.closeRequest.by),
          date: day(summary.closeRequest.at),
        }),
      )
    }
  }

  return {
    heading: w.heading,
    intro: w.intro,
    between: fmt(w.between, { a: name('A'), b: name('B') }),
    basis,
    sides,
    signed,
    standing,
  }
}
