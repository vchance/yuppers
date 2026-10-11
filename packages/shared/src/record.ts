import type { components, RevisionTerms } from '@yuppers/api-client'

import type { MessageValues } from './message'
import type {
  ContributionEventType,
  FormerClaimantEventType,
  NeutralEventType,
  PartyEventType,
  RecordNoteKind,
  Wording,
} from './wording/types'

type Schemas = components['schemas']
export type RecordDocument = Schemas['RecordDocument']
export type RecordEvent = Schemas['RecordEvent']
export type RecordRevision = Schemas['RecordRevision']
export type SignedDocument = Schemas['SignedDocument']
export type HistoryPage = Schemas['HistoryPage']
type Continuation = Schemas['Continuation']
type Parties = Schemas['Parties']
type Slot = Schemas['Slot']

/*
 * Reading an exchange's record: its history, and the copy a party keeps
 * (DESIGN.md §14.1). The service decides what the record holds; this only
 * puts it into words and puts its parts together.
 */

const NEUTRAL: ReadonlySet<string> = new Set<NeutralEventType>([
  'COUNTERPARTY_RELEASED',
  'REVISION_SUPERSEDED',
  'REVISION_EXPIRED',
  'AGREEMENT_IN_FORCE',
  'INACTIVITY_PROMPTED',
])

/**
 * The sentence for one event, as a wording message and the values to fill it
 * with. `reader` is the party reading, who is addressed as "you" for what
 * they did themselves; with `null` everyone is named, as on a page that may
 * be handed to someone else.
 *
 * The contribution an event is about and any note written with it are not
 * part of the sentence. They are the parties' own words, shown as such.
 *
 * `money` is the ids of the money contributions, which are paid outside the
 * product and only recorded here, so what was done about one is said in
 * words for paying and receiving rather than delivering.
 */
export function eventMessage(
  event: RecordEvent,
  words: Wording['record']['events'],
  reader: Slot | null,
  parties: Parties,
  money?: ReadonlySet<string>,
): { message: string; values: MessageValues } {
  const values: MessageValues = {}
  if (event.revision) values.number = event.revision.sequence

  if (event.type === 'EXCHANGE_CLOSED') {
    // An exchange that closes always says how. One that somehow did not is
    // described by the least it could mean.
    return { message: words.closed[event.outcome ?? 'UNRESOLVED'], values }
  }
  if (NEUTRAL.has(event.type)) {
    return { message: words.neutral[event.type as NeutralEventType], values }
  }
  // Done from the invited party's place by someone who was later removed
  // from it, or left (DESIGN.md §8). The record gives that place a name, and
  // it is not theirs; nor are they the reader, even if the reader is in that
  // place now. Checked against the wording's own keys: an exchange older
  // than the rule could hold something else of theirs, which is then said
  // the ordinary way. (`in` would also find what every object inherits,
  // such as `toString`.)
  if (event.by_removed_claimant && Object.hasOwn(words.formerClaimant, event.type)) {
    return { message: words.formerClaimant[event.type as FormerClaimantEventType], values }
  }
  const type = event.type as PartyEventType
  const aboutMoney =
    event.contribution !== undefined &&
    event.contribution !== null &&
    money?.has(event.contribution.id) === true &&
    Object.hasOwn(words.moneyNamed, type)
  if (event.actor !== 'SYSTEM' && event.actor === reader) {
    return {
      message: aboutMoney ? words.moneyYou[type as ContributionEventType] : words.you[type],
      values,
    }
  }
  if (event.actor !== 'SYSTEM') values.name = parties[event.actor]
  return {
    message: aboutMoney ? words.moneyNamed[type as ContributionEventType] : words.named[type],
    values,
  }
}

/** What the note written with an event is, so it can be labelled. */
export function noteKind(event: RecordEvent): RecordNoteKind {
  switch (event.type) {
    case 'REVISION_SENT':
      return 'message'
    case 'CONTRIBUTION_DISPUTED':
      return 'reason'
    case 'CLOSE_REQUESTED':
    case 'STATEMENT_ADDED':
      return 'statement'
    case 'PROGRESS_NOTED':
      return 'progress'
    default:
      return 'note'
  }
}

/**
 * What a revision in the record says: the document its signatures cover, or,
 * when a reviewer has hidden what was written in the exchange from the
 * reader, the redacted copy the record gives in its place (`content_hidden`
 * says so). One of the two is always there. Only `signed` hashes to the
 * revision's `content_hash`.
 */
export function documentOf(revision: RecordRevision): SignedDocument {
  return (revision.signed ?? revision.redacted) as SignedDocument
}

/** What a revision in the record says, in the shape the terms are read in everywhere else. */
export function termsOfRevision(revision: RecordRevision): RevisionTerms {
  const signed = documentOf(revision)
  return {
    party_a_name: signed.parties.A,
    party_b_name: signed.parties.B,
    terms: signed.terms,
    contributions: signed.contributions.map((contribution) => ({
      id: contribution.id,
      from: contribution.from,
      type: contribution.type,
      description: contribution.description,
      quantity: contribution.quantity,
      due: contribution.due,
      completion_criteria: contribution.completion_criteria,
      required: contribution.required,
      amount_minor: contribution.amount_minor,
    })),
  }
}

/**
 * Puts the parts of a long record together into one document. The parts must
 * be consecutive, starting with the first. How the exchange stands is taken
 * from the last part, which was read last.
 */
export function joinRecord(parts: readonly RecordDocument[]): RecordDocument {
  const first = parts[0]
  const last = parts[parts.length - 1]
  if (parts.length === 1) return first
  const whole = first.part.from.revisions_after === 0 && first.part.from.events_after === 0
  return {
    ...last,
    revisions: parts.flatMap((part) => part.revisions),
    events: parts.flatMap((part) => part.events),
    part: {
      from: first.part.from,
      next: last.part.next ?? null,
      complete: whole && !last.part.next,
    },
  }
}

/** More parts than this is not a record anyone reads on one page; stop asking. */
const MOST_PARTS = 200

/**
 * Reads a whole record, however many parts it comes in. `read` fetches one
 * part: the first when given `null`, otherwise the one that starts where the
 * part before said the next would.
 *
 * Something can happen in the exchange between two parts. The parts then
 * still join up, since history is only ever added to, but what an early part
 * said about a revision may be out of date; so the record is read again, a
 * couple of times at most.
 */
export async function readWholeRecord(
  read: (from: Continuation | null) => Promise<RecordDocument>,
): Promise<RecordDocument> {
  for (let attempt = 1; ; attempt += 1) {
    const parts = [await read(null)]
    let next = parts[0].part.next
    while (next && parts.length < MOST_PARTS) {
      const part = await read(next)
      parts.push(part)
      next = part.part.next
    }
    const settled = parts[0].exchange.last_event === parts[parts.length - 1].exchange.last_event
    if (settled || attempt === 3) return joinRecord(parts)
  }
}

/**
 * Writes a moment with its seconds and its time zone, in the exchange's own
 * zone: a record is read later and by other people, so a time in it cannot
 * depend on where its reader happens to be.
 */
export function recordMoments(language: string, timezone: string): (instant: string) => string {
  const style = { dateStyle: 'long', timeStyle: 'long' } as const
  let format: Intl.DateTimeFormat
  try {
    format = new Intl.DateTimeFormat(language, { ...style, timeZone: timezone })
  } catch {
    // A zone this device does not know. The zone is written with each time.
    format = new Intl.DateTimeFormat(language, { ...style, timeZone: 'UTC' })
  }
  return (instant) => {
    const parsed = new Date(instant)
    return Number.isNaN(parsed.getTime()) ? instant : format.format(parsed)
  }
}

/**
 * Writes a moment as its calendar date in the exchange's own zone, for the
 * plain summary at the top of a record, which says on what day things
 * happened and leaves the exact time to the record under it.
 */
export function recordDays(language: string, timezone: string): (instant: string) => string {
  let format: Intl.DateTimeFormat
  try {
    format = new Intl.DateTimeFormat(language, { dateStyle: 'long', timeZone: timezone })
  } catch {
    format = new Intl.DateTimeFormat(language, { dateStyle: 'long', timeZone: 'UTC' })
  }
  return (instant) => {
    const parsed = new Date(instant)
    return Number.isNaN(parsed.getTime()) ? instant : format.format(parsed)
  }
}

/**
 * How a signer was verified, in the reader's language. A method newer than
 * this build has no wording here and is described by the record itself.
 */
export function verificationText(
  verification: Schemas['Verification'],
  words: Wording['record']['export']['verification'],
): string {
  const known: Partial<Record<string, string>> = words
  return known[verification.method] ?? verification.description
}

/** A record as a file: the copy a party takes away (DESIGN.md §14.1). */
export interface RecordFile {
  /** With its extension. */
  name: string
  type: 'application/json'
  text: string
}

/**
 * The record as one JSON file, the same whichever client hands it over.
 * `name` is the file's name without its extension, from the wording.
 */
export function recordFile(record: RecordDocument, name: string): RecordFile {
  return {
    name: `${name}.json`,
    type: 'application/json',
    text: `${JSON.stringify(record, null, 2)}\n`,
  }
}
