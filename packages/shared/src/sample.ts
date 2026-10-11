import type { components, RevisionTerms } from '@yuppers/api-client'

import { todayIn } from './fulfillment'
import type { RecordDocument, RecordEvent } from './record'
import type { Wording } from './wording/types'

type Status = components['schemas']['Status']
type Parties = components['schemas']['Parties']

/*
 * The sample yup (DESIGN.md §4.3): a fence repair between two made-up
 * neighbours, in force and part-way through, so the whole lifecycle can be
 * seen at once. A fixed document that ships with the apps and is drawn by
 * the components that draw real yups. No service is involved, nothing is
 * stored, and the people in it are never the reader.
 *
 * Its words are the wording files' (`sample`), in every language, since it is
 * product content and not a party's own. Its dates are worked out from today
 * when it is shown, so that it never looks stale: signed three days ago, the
 * repair due in six days.
 */

export const SAMPLE_CURRENCY = 'USD'

export const SAMPLE_IDS = {
  exchange: '5a3e0000-0000-4000-8000-000000000000',
  revision: '5a3e0000-0000-4000-8000-0000000000a1',
  deposit: '5a3e0000-0000-4000-8000-000000000001',
  repair: '5a3e0000-0000-4000-8000-000000000002',
  balance: '5a3e0000-0000-4000-8000-000000000003',
} as const

/** Days before today that the sample was signed, and after today that the repair is due. */
export const SAMPLE_SIGNED_DAYS_AGO = 3
export const SAMPLE_DUE_IN_DAYS = 6

export interface Sample {
  parties: Parties
  terms: RevisionTerms
  currency: string
  timezone: string
  /** Where each item stands, by id. */
  statuses: ReadonlyMap<string, Status>
  /** The ids of the money items, which are spoken of in words for paying and receiving. */
  money: ReadonlySet<string>
  record: RecordDocument
  /** The record's events, oldest first, for the history. */
  events: readonly RecordEvent[]
  /** The date the repair is due, `YYYY-MM-DD`. */
  repairDue: string
}

/** A calendar date, `YYYY-MM-DD`, some days from another. */
function addDays(date: string, days: number): string {
  const [year, month, day] = date.split('-').map(Number)
  return new Date(Date.UTC(year, month - 1, day + days)).toISOString().slice(0, 10)
}

/** The instant a number of days before `now`, at a fixed hour of that day. */
function daysAgo(now: Date, days: number, hour: number, minute = 0): string {
  const at = new Date(now)
  at.setUTCDate(at.getUTCDate() - days)
  at.setUTCHours(hour, minute, 0, 0)
  return at.toISOString().replace('.000Z', 'Z')
}

/**
 * The sample as of `now`, in `timezone` (the reader's own, whose today is
 * what "six days from today" means) and in the language `words` is in.
 */
export function sampleYup(
  now: Date,
  timezone: string,
  words: Wording['sample'],
  language: string,
): Sample {
  const parties: Parties = { A: words.partyA, B: words.partyB }
  const repairDue = addDays(todayIn(timezone, now), SAMPLE_DUE_IN_DAYS)

  const contributions: RevisionTerms['contributions'] = [
    {
      id: SAMPLE_IDS.deposit,
      from: 'B',
      type: 'MONEY',
      description: words.deposit,
      quantity: null,
      due: { kind: 'ON_AGREEMENT' },
      completion_criteria: null,
      required: true,
      amount_minor: 10_000,
    },
    {
      id: SAMPLE_IDS.repair,
      from: 'A',
      type: 'SERVICE',
      description: words.repair,
      quantity: null,
      due: { kind: 'DATE', date: repairDue },
      completion_criteria: words.repairDone,
      required: true,
      amount_minor: null,
    },
    {
      id: SAMPLE_IDS.balance,
      from: 'B',
      type: 'MONEY',
      description: words.balance,
      quantity: null,
      due: { kind: 'AFTER_CONTRIBUTION', contribution: SAMPLE_IDS.repair },
      completion_criteria: null,
      required: true,
      amount_minor: 30_000,
    },
  ]
  const terms: RevisionTerms = {
    party_a_name: words.partyA,
    party_b_name: words.partyB,
    terms: words.terms,
    contributions,
  }

  const statuses = new Map<string, Status>([
    [SAMPLE_IDS.deposit, 'ACCEPTED'],
    [SAMPLE_IDS.repair, 'PENDING'],
    [SAMPLE_IDS.balance, 'PENDING'],
  ])
  const money = new Set<string>([SAMPLE_IDS.deposit, SAMPLE_IDS.balance])

  const sentAt = daysAgo(now, SAMPLE_SIGNED_DAYS_AGO, 15, 0)
  const openedAt = daysAgo(now, SAMPLE_SIGNED_DAYS_AGO, 16, 0)
  const signedAt = daysAgo(now, SAMPLE_SIGNED_DAYS_AGO, 16, 10)
  const paidAt = daysAgo(now, 2, 14, 30)
  const confirmedAt = daysAgo(now, 2, 18, 5)
  const revision = { id: SAMPLE_IDS.revision, sequence: 1 }
  const deposit = { id: SAMPLE_IDS.deposit, description: words.deposit }

  const happened: Omit<RecordEvent, 'sequence'>[] = [
    { type: 'REVISION_SENT', actor: 'A', at: sentAt, revision },
    { type: 'COUNTERPARTY_CLAIMED', actor: 'B', at: openedAt },
    { type: 'REVISION_ACCEPTED', actor: 'B', at: signedAt, revision },
    { type: 'AGREEMENT_IN_FORCE', actor: 'B', at: signedAt, revision },
    {
      type: 'CONTRIBUTION_CLAIMED',
      actor: 'B',
      at: paidAt,
      revision,
      contribution: deposit,
      status: 'CLAIMED',
    },
    {
      type: 'CONTRIBUTION_CONFIRMED',
      actor: 'A',
      at: confirmedAt,
      revision,
      contribution: deposit,
      status: 'ACCEPTED',
    },
  ]
  const events = happened.map((event, index) => ({ ...event, sequence: index + 1 }))

  const verified = (at: string) => ({
    method: 'EMAIL_OTP' as const,
    verified_at: at,
    description: '',
  })
  const record: RecordDocument = {
    format: 'exchange-record',
    format_version: 1,
    generated_at: new Date(now).toISOString().replace(/\.\d+Z$/, 'Z'),
    language,
    // The sample says what a real record would say here in its own banner and
    // its own line (`sample.fingerprint`); these notices are not shown.
    notices: { about: '', signatures: '', statements: '', content_hash: '' },
    prepared_for: 'A',
    exchange: {
      id: SAMPLE_IDS.exchange,
      display_code: '',
      timezone,
      currency: SAMPLE_CURRENCY,
      created_at: sentAt,
      state: 'ACTIVE',
      counterparty: 'CONFIRMED',
      in_force_revision: revision,
      end_proposed_by: null,
      last_event: events.length,
    },
    parties,
    contributions: contributions.map((item) => ({
      id: item.id,
      from: item.from,
      description: item.description,
      required: item.required,
      status: statuses.get(item.id) ?? 'PENDING',
      since: item.id === SAMPLE_IDS.deposit ? confirmedAt : signedAt,
    })),
    revisions: [
      {
        id: SAMPLE_IDS.revision,
        sequence: 1,
        author: 'A',
        sent_at: sentAt,
        expires_at: addDays(sentAt.slice(0, 10), 14) + 'T00:00:00Z',
        standing: { status: 'IN_FORCE', since: signedAt, in_force_at: signedAt },
        note: null,
        content_hash: '',
        signed: {
          v: 1,
          exchange: SAMPLE_IDS.exchange,
          currency: SAMPLE_CURRENCY,
          timezone,
          parties,
          terms: terms.terms,
          attachments: [],
          contributions: contributions.map((item) => ({
            id: item.id,
            from: item.from,
            type: item.type,
            description: item.description,
            quantity: item.quantity ?? null,
            due: item.due,
            completion_criteria: item.completion_criteria ?? null,
            required: item.required,
            amount_minor: item.amount_minor ?? null,
            settlement: item.type === 'MONEY' ? 'OFF_PLATFORM' : null,
          })),
        },
        signatures: [
          {
            party: 'A',
            name: words.partyA,
            signed_at: sentAt,
            content_hash: '',
            verification: verified(sentAt),
            consent: { language, version: '' },
          },
          {
            party: 'B',
            name: words.partyB,
            signed_at: signedAt,
            content_hash: '',
            verification: verified(openedAt),
            consent: { language, version: '' },
          },
        ],
      },
    ],
    events,
    part: { from: { revisions_after: 0, events_after: 0 }, next: null, complete: true },
  }

  return { parties, terms, currency: SAMPLE_CURRENCY, timezone, statuses, money, record, events, repairDue }
}
