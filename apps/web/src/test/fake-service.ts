import type { Account, ErrorCode, ExchangeSummary, ExchangeView } from '@yuppers/api-client'
import type { HistoryPage, RecordDocument, RevisionView } from '@yuppers/shared'

/*
 * A stand-in for the service, for rendering the web app's screens in a test
 * with the state a person would see them in. It answers the calls those
 * screens make and nothing else; the session is the cookie the browser
 * would hold, so here it is simply whether `account` is set.
 */

export const ACTIVE = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70'
export const DRAFT = '5d0c3b1a-2f64-4e8b-9a7d-1c2e3f4a5b6c'
export const OFFER = '7e2f3a4b-5c6d-4e7f-8a9b-0c1d2e3f4a5b'
/** A counteroffer from Ben, waiting for Ana. */
export const COUNTER = '8a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d'
/** An amendment from Ana, waiting for Ben. */
export const AMENDING = '9b2c3d4e-5f6a-4b7c-9d8e-0f1a2b3c4d5e'
/** An agreement in force where Ben has disputed the repair. */
export const DISPUTED = 'ac3d4e5f-6a7b-4c8d-8e9f-1a2b3c4d5e6f'
/** An exchange that ended by agreement. */
export const ENDED = 'bd4e5f6a-7b8c-4d9e-9f0a-2b3c4d5e6f7a'
export const REPAIR = '11111111-1111-4111-8111-111111111111'
export const PAYMENT = '22222222-2222-4222-8222-222222222222'
export const GATE = '33333333-3333-4333-8333-333333333333'
export const INVITATION = 'a3'.repeat(32)
/** A second invitation, to another proposal, whose preview carries its own reference. */
export const OTHER_INVITATION = 'b4'.repeat(32)
export const OTHER_INVITATION_CODE = 'OTHR-5K8P'
/** The one code the stand-in accepts. */
export const GOOD_CODE = '123456'

/** The token of the invitation a first proposal sent from the draft gets. */
export const SENT_INVITATION = 'c5'.repeat(32)

/** The address the stand-in says codes come from, unless a test says otherwise. */
export const CODE_SENDER = 'codes@yuppers.example'

export const ana: Account = {
  id: 'a0000000-0000-4000-8000-000000000001',
  display_name: 'Ana Ruiz',
  adult_confirmed: true,
  language: 'en',
  email: 'ana@example.test',
}

/** A reviewer of abuse reports (DESIGN.md §9). Only she gets anything but "not found" from the staff paths. */
export const rita: Account = {
  id: 'a0000000-0000-4000-8000-000000000009',
  display_name: 'Rita Reviewer',
  adult_confirmed: true,
  language: 'en',
  email: 'rita@example.test',
}

/** A reviewer whose sign-in is more than twelve hours old. */
export const staleRita: Account = { ...rita, id: 'a0000000-0000-4000-8000-00000000000a' }

/** A report another reviewer has already resolved. */
export const RESOLVED_REPORT = 'f0000000-0000-4000-8000-000000000003'

/** An open report about the agreement in force, filed by Ben about Ana. */
export const REPORT = 'f0000000-0000-4000-8000-000000000001'
/** An open report older than a day. */
export const OLD_REPORT = 'f0000000-0000-4000-8000-000000000002'
const BEN_ID = 'b0000000-0000-4000-8000-000000000002'

const PARTIES = { A: 'Ana Ruiz', B: 'Ben Ortiz' }

/**
 * Text in the data below that may show on a screen without coming from the
 * wording (`pseudo.test.tsx`): what people wrote or chose, and what the
 * service writes in the reader's language. Keep it in step with the data.
 */
export const STAND_IN_TEXT: readonly string[] = [
  'Ana Ruiz',
  'Ben Ortiz',
  'Here is what we talked about on Tuesday.',
  'Repair the back fence and the gate',
  'Repair the back fence.',
  'Repair the back fence',
  'The gate closes and latches.',
  'Payment for the repair',
  'Paint the gate',
  'Finished on Monday.',
  // The exchange's currency and time zone, chosen by whoever started it.
  'USD',
  'America/Chicago',
  // The record's notices and how a signature was checked come from the service.
  'about, from the service',
  'signatures, from the service',
  'statements, from the service',
  'content hash, from the service',
  'described by the record',
  // The version of the consent wording a signature was given under.
  'draft-1',
  // What a reporter and a reviewer wrote.
  'She threatened me in a note.',
  'Threats in the notes.',
]

export const revision: RevisionView = {
  id: 'c0000000-0000-4000-8000-000000000001',
  sequence: 1,
  author: 'A',
  accepted_by: ['A', 'B'],
  content_hash: 'ab'.repeat(32),
  expires_at: '2026-10-16T12:00:00Z',
  note: 'Here is what we talked about on Tuesday.',
  terms: {
    party_a_name: PARTIES.A,
    party_b_name: PARTIES.B,
    terms: 'Repair the back fence.',
    contributions: [
      {
        id: REPAIR,
        from: 'A',
        type: 'SERVICE',
        description: 'Repair the back fence',
        due: { kind: 'DATE', date: '2026-10-30' },
        completion_criteria: 'The gate closes and latches.',
        required: true,
      },
      {
        id: PAYMENT,
        from: 'B',
        type: 'MONEY',
        description: 'Payment for the repair',
        due: { kind: 'AFTER_CONTRIBUTION', contribution: REPAIR },
        required: true,
        amount_minor: 45000,
      },
    ],
  },
}

const common = {
  currency: 'USD',
  timezone: 'America/Chicago',
  counterparty: 'CONFIRMED' as const,
}

/** An agreement in force, read by the person who started it. */
export function activeExchange(): ExchangeView {
  return {
    ...common,
    id: ACTIVE,
    version: 7,
    state: 'ACTIVE',
    you: 'A',
    display_code: 'PVVS-5Q2K',
    in_force_revision: revision,
    contributions: [
      { id: REPAIR, status: 'CLAIMED', since: '2026-10-20T14:30:00Z' },
      { id: PAYMENT, status: 'PENDING' },
    ],
  }
}

/** A proposal waiting for the reader, the invited party, to sign it. */
export function offerExchange(): ExchangeView {
  return {
    ...common,
    id: OFFER,
    version: 3,
    state: 'NEGOTIATING',
    you: 'B',
    display_code: 'OFFR-7Y2M',
    open_revision: { ...revision, accepted_by: ['A'] },
    contributions: [],
  }
}

/** A first proposal being written. */
export function draftExchange(): ExchangeView {
  return {
    ...common,
    id: DRAFT,
    version: 1,
    state: 'DRAFT',
    you: 'A',
    counterparty: 'UNCLAIMED',
    display_code: 'DRFT-0001',
    contributions: [],
    draft: {
      format: 1,
      base: null,
      partyA: PARTIES.A,
      partyB: PARTIES.B,
      terms: '',
      note: '',
      contributions: [
        {
          id: REPAIR,
          from: 'A',
          type: 'SERVICE',
          description: 'Repair the back fence',
          quantity: '',
          unit: '',
          due: { kind: 'DATE', date: '2026-10-30' },
          criteria: '',
          required: true,
          amount: '',
        },
        {
          id: PAYMENT,
          from: 'B',
          type: 'MONEY',
          description: 'Payment for the repair',
          quantity: '',
          unit: '',
          due: { kind: 'AFTER_CONTRIBUTION', contribution: REPAIR },
          criteria: '',
          required: true,
          amount: '450',
        },
      ],
    } as never,
  }
}

/** The draft once its first proposal is sent: open, with nobody invited yet. */
function sentExchange(): ExchangeView {
  const { draft: _draft, ...sent } = draftExchange()
  return {
    ...sent,
    version: 2,
    state: 'NEGOTIATING',
    open_revision: { ...revision, accepted_by: ['A'] },
  }
}

/** Ben's answer to the first version: a higher payment, and a gate to paint. */
export const counterRevision: RevisionView = {
  ...revision,
  id: 'c0000000-0000-4000-8000-000000000002',
  sequence: 2,
  author: 'B',
  accepted_by: ['B'],
  note: null,
  terms: {
    ...revision.terms,
    contributions: [
      revision.terms.contributions[0],
      { ...revision.terms.contributions[1], amount_minor: 50000 },
      {
        id: GATE,
        from: 'A',
        type: 'TASK',
        description: 'Paint the gate',
        due: { kind: 'ON_AGREEMENT' },
        required: false,
      },
    ],
  },
}

/** Ana's change to the agreement: the repair is described anew. */
export const amendmentRevision: RevisionView = {
  ...revision,
  id: 'c0000000-0000-4000-8000-000000000003',
  sequence: 2,
  author: 'A',
  accepted_by: ['A'],
  note: null,
  terms: {
    ...revision.terms,
    contributions: [
      { ...revision.terms.contributions[0], description: 'Repair the back fence and the gate' },
      revision.terms.contributions[1],
    ],
  },
}

function counterExchange(): ExchangeView {
  return {
    ...common,
    id: COUNTER,
    version: 5,
    state: 'NEGOTIATING',
    you: 'A',
    display_code: 'CNTR-4H7J',
    open_revision: counterRevision,
    contributions: [],
  }
}

function amendingExchange(): ExchangeView {
  return {
    ...activeExchange(),
    id: AMENDING,
    you: 'B',
    display_code: 'AMND-2X9Q',
    open_revision: amendmentRevision,
  }
}

function disputedExchange(): ExchangeView {
  return {
    ...activeExchange(),
    id: DISPUTED,
    you: 'B',
    display_code: 'DSPT-8M3R',
    contributions: [
      { id: REPAIR, status: 'DISPUTED', since: '2026-10-21T09:00:00Z' },
      { id: PAYMENT, status: 'PENDING' },
    ],
  }
}

function endedExchange(): ExchangeView {
  return {
    ...activeExchange(),
    id: ENDED,
    state: 'CLOSED',
    closed_outcome: 'ENDED_BY_AGREEMENT',
    display_code: 'ENDD-6T1W',
    contributions: [
      { id: REPAIR, status: 'ACCEPTED', since: '2026-10-21T09:00:00Z' },
      { id: PAYMENT, status: 'WAIVED', since: '2026-10-24T09:00:00Z' },
    ],
  }
}

const exchanges = (): ExchangeView[] => [activeExchange(), offerExchange(), draftExchange()]
/** Exchanges that can be opened but are not in the list, so the list stays as it was. */
const others = (): ExchangeView[] => [
  counterExchange(),
  amendingExchange(),
  disputedExchange(),
  endedExchange(),
]

function summary(exchange: ExchangeView): ExchangeSummary {
  return {
    id: exchange.id,
    display_code: exchange.display_code,
    other_party_name:
      exchange.state === 'DRAFT' ? '' : exchange.you === 'A' ? PARTIES.B : PARTIES.A,
    state: exchange.state,
    updated_at: '2026-10-02T06:30:00Z',
    you: exchange.you,
  }
}

function history(exchange: ExchangeView): HistoryPage {
  const at = '2026-10-02T15:00:05Z'
  const sent = { id: revision.id, sequence: 1 }
  return {
    you: exchange.you,
    parties: PARTIES,
    earlier: null,
    events: [
      { sequence: 1, type: 'REVISION_SENT', actor: 'A', at, revision: sent, note: revision.note },
      ...(exchange.state === 'ACTIVE'
        ? [
            {
              sequence: 2,
              type: 'REVISION_ACCEPTED' as const,
              actor: 'B' as const,
              at,
              revision: sent,
            },
            {
              sequence: 3,
              type: 'CONTRIBUTION_CLAIMED' as const,
              actor: 'A' as const,
              at: '2026-10-20T14:30:00Z',
              revision: sent,
              contribution: { id: REPAIR, description: 'Repair the back fence' },
              status: 'CLAIMED' as const,
              note: 'Finished on Monday.',
            },
          ]
        : []),
    ],
  }
}

function record(exchange: ExchangeView): RecordDocument {
  const signedAt = '2026-10-02T16:10:00Z'
  const verification = (method: 'EMAIL_OTP' | 'PHONE_OTP') => ({
    method,
    verified_at: '2026-10-02T15:58:00Z',
    description: 'described by the record',
  })
  const counter = exchange.id === COUNTER
  const amending = exchange.id === AMENDING
  const ended = exchange.id === ENDED
  /** One version as the record holds it. */
  const recorded = (
    view: RevisionView,
    standing: RecordDocument['revisions'][number]['standing'],
    signers: readonly ('A' | 'B')[],
  ): RecordDocument['revisions'][number] => ({
    id: view.id,
    sequence: view.sequence,
    author: view.author,
    sent_at: '2026-10-02T15:00:05Z',
    expires_at: view.expires_at,
    standing,
    note: view.note,
    answers: view.sequence > 1 ? { id: revision.id, sequence: 1 } : null,
    content_hash: view.content_hash,
    signed: {
      v: 1,
      exchange: exchange.id,
      currency: exchange.currency,
      timezone: exchange.timezone,
      parties: PARTIES,
      terms: view.terms.terms,
      attachments: [],
      contributions: view.terms.contributions.map((item) => ({
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
    signatures: signers.map((party) => ({
      party,
      name: PARTIES[party],
      signed_at: signedAt,
      content_hash: view.content_hash,
      verification: verification(party === 'A' ? 'EMAIL_OTP' : 'PHONE_OTP'),
      consent: { language: 'en', version: 'draft-1' },
    })),
  })
  const revisions = counter
    ? [
        recorded(revision, { status: 'SUPERSEDED', since: signedAt }, ['A']),
        recorded(counterRevision, { status: 'OPEN', since: signedAt }, ['B']),
      ]
    : [
        recorded(revision, { status: 'IN_FORCE', since: signedAt, in_force_at: signedAt }, ['A', 'B']),
        ...(amending ? [recorded(amendmentRevision, { status: 'OPEN', since: signedAt }, ['A'])] : []),
      ]
  const closedAt = '2026-10-24T09:00:00Z'
  const events = history(exchange).events
  if (ended) {
    const at = closedAt
    events.push(
      {
        sequence: 4,
        type: 'CONTRIBUTION_CONFIRMED',
        actor: 'B',
        at: '2026-10-21T09:00:00Z',
        contribution: { id: REPAIR, description: 'Repair the back fence' },
        status: 'ACCEPTED',
      },
      { sequence: 5, type: 'END_PROPOSED', actor: 'B', at },
      {
        sequence: 6,
        type: 'EXCHANGE_CLOSED',
        actor: 'A',
        at,
        outcome: 'ENDED_BY_AGREEMENT',
        waived: [PAYMENT],
      },
    )
  }
  return {
    format: 'exchange-record',
    format_version: 2,
    generated_at: '2026-10-22T18:00:00Z',
    language: 'en',
    notices: {
      about: 'about, from the service',
      signatures: 'signatures, from the service',
      statements: 'statements, from the service',
      content_hash: 'content hash, from the service',
    },
    prepared_for: 'A',
    exchange: {
      id: exchange.id,
      display_code: exchange.display_code,
      timezone: exchange.timezone,
      currency: exchange.currency,
      created_at: '2026-10-02T14:50:00Z',
      state: exchange.state,
      counterparty: exchange.counterparty,
      in_force_revision: counter ? null : { id: revision.id, sequence: 1 },
      open_revision: exchange.open_revision
        ? { id: exchange.open_revision.id, sequence: exchange.open_revision.sequence }
        : null,
      closed_outcome: exchange.closed_outcome ?? null,
      closed_at: exchange.state === 'CLOSED' ? closedAt : null,
      last_event: events.length,
    },
    parties: PARTIES,
    contributions: counter
      ? []
      : revision.terms.contributions.map((item) => ({
          id: item.id,
          from: item.from,
          description: item.description,
          required: item.required,
          status:
            exchange.contributions.find((stands) => stands.id === item.id)?.status ?? 'PENDING',
          since: signedAt,
        })),
    revisions,
    events,
    part: { from: { revisions_after: 0, events_after: 0 }, next: null, complete: true },
  } as RecordDocument
}

export interface FakeService {
  /** Who the session cookie belongs to; `null` when nobody is signed in. */
  account: Account | null
  /** Every request so far, as `METHOD /path` with its body, oldest first. */
  sent: { call: string; body: unknown }[]
  /** Whether the service says it can send codes to phone numbers. */
  phone: boolean
  /** The address the service says codes come from, if it says. */
  codeSender: string | null
  /** Whether the draft's first proposal has been sent. */
  proposed: boolean
  /** A refusal for every request for a code from now on, such as a limit. */
  refuseCodes: ErrorCode | null
  /** Whether the service texts agreement updates (`sms_updates` in its meta). */
  texting: boolean
  /** The agreements the account has turned text updates on for. */
  textUpdates: Set<string>
  /** Whether the account's number replied STOP. */
  optedOut: boolean
  fetch: typeof fetch
}

export function fakeService(account: Account | null): FakeService {
  const service: FakeService = {
    account,
    sent: [],
    phone: true,
    codeSender: CODE_SENDER,
    proposed: false,
    refuseCodes: null,
    texting: true,
    textUpdates: new Set(),
    optedOut: false,
    fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
      const request = input instanceof Request ? input : new Request(input, init)
      const text = await request.text()
      const body: unknown = text ? JSON.parse(text) : null
      const path = new URL(request.url).pathname
      const call = `${request.method} ${path}`
      service.sent.push({ call, body })
      const [status, answer] = respond(service, call, body)
      return new Response(answer === null ? null : JSON.stringify(answer), {
        status,
        headers: answer === null ? {} : { 'Content-Type': 'application/json' },
      })
    }) as typeof fetch,
  }
  return service
}

/** A report as the queue lists it: nothing of what the reporter wrote, or who anyone is. */
function queuedReport(id: string, hours: number, overdue: boolean) {
  return {
    id,
    created_at: '2026-10-22T09:00:00Z',
    age_seconds: hours * 3600,
    overdue,
    reason: overdue ? ('SCAM' as const) : ('HARASSMENT' as const),
    has_reporter: !overdue,
    display_code: 'PVVS-5Q2K',
  }
}

/** A report as its page shows it, once opened. */
function openedReport(id: string, hours: number, overdue: boolean) {
  return {
    id,
    created_at: '2026-10-22T09:00:00Z',
    age_seconds: hours * 3600,
    overdue,
    reason: overdue ? ('SCAM' as const) : ('HARASSMENT' as const),
    details: overdue ? null : 'She threatened me in a note.',
    reporter_account_id: overdue ? null : BEN_ID,
    subject_account_id: ana.id,
    exchange_id: ACTIVE,
    display_code: 'PVVS-5Q2K',
  }
}

/**
 * A name written right to left, with a character that would turn the rest
 * of its line around: the staff screen must keep it from steering the words
 * around it.
 */
export const RTL_NAME = 'مريم\u202E الحداد'

/** The staff review calls, for a reviewer. */
function staff(call: string): [number, unknown] | null {
  if (call === 'GET /v1/staff/reports') {
    return [
      200,
      {
        review_within_hours: 24,
        reports: [queuedReport(OLD_REPORT, 30, true), queuedReport(REPORT, 2, false)],
      },
    ]
  }
  if (call === `GET /v1/staff/reports/${REPORT}`) {
    const document = record(activeExchange())
    return [
      200,
      {
        report: openedReport(REPORT, 2, false),
        reporter: { id: BEN_ID, status: 'ACTIVE', party: 'B', name: PARTIES.B },
        subject: { id: ana.id, status: 'ACTIVE', party: 'A', name: PARTIES.A },
        content_hidden: false,
        record: {
          exchange: document.exchange,
          parties: document.parties,
          contributions: document.contributions,
          revisions: document.revisions,
          events: document.events,
          complete: true,
        },
        other_reports: [
          {
            id: OLD_REPORT,
            created_at: '2026-10-21T09:00:00Z',
            reason: 'SCAM',
            status: 'OPEN',
            outcome: null,
          },
        ],
        history: [
          {
            id: 1,
            action: 'REPORT_VIEWED',
            staff_account_id: rita.id,
            at: '2026-10-22T10:00:00Z',
            note: null,
            report_id: REPORT,
            exchange_id: ACTIVE,
            account_id: ana.id,
          },
        ],
      },
    ]
  }
  if (call === `POST /v1/staff/reports/${REPORT}/resolution`) return [204, null]
  if (call === `GET /v1/staff/reports/${RESOLVED_REPORT}`) return [409, { code: 'REPORT_RESOLVED' }]
  if (call === 'GET /v1/staff/suspensions') {
    return [
      200,
      [
        {
          account_id: BEN_ID,
          name: PARTIES.B,
          suspended_at: '2026-10-20T09:00:00Z',
          note: 'Threats in the notes.',
          report_id: OLD_REPORT,
        },
      ],
    ]
  }
  if (call === `POST /v1/staff/suspensions/${BEN_ID}/lift`) return [204, null]
  if (call === 'GET /v1/staff/hidden') {
    return [
      200,
      [
        {
          exchange_id: ACTIVE,
          display_code: 'PVVS-5Q2K',
          account_id: ana.id,
          name: RTL_NAME,
          hidden_at: '2026-10-21T09:00:00Z',
          report_id: OLD_REPORT,
        },
      ],
    ]
  }
  return null
}

/** Where the account stands on text updates for an agreement. */
function textUpdates(service: FakeService, exchange: ExchangeView) {
  return {
    on: service.textUpdates.has(exchange.id),
    available: service.texting && ['NEGOTIATING', 'ACTIVE'].includes(exchange.state),
    phone: service.account?.phone ?? null,
    opted_out: service.optedOut,
    consent_version: '2026-10-05',
  }
}

function respond(service: FakeService, call: string, body: unknown): [number, unknown] {
  if (call === 'GET /v1/meta') {
    return [
      200,
      {
        service: 'yuppers-backend',
        version: '0.0.0',
        commit: 'unknown',
        built_at: null,
        minimum_client_versions: { web: null, ios: null, android: null },
        // Both, so a device's own wallet button shows on an agreement in force.
        wallet_platforms: ['APPLE', 'GOOGLE'],
        sign_in_channels: service.phone ? ['email', 'phone'] : ['email'],
        sms_country_codes: service.phone ? ['+1'] : [],
        sms_updates: service.texting,
        ...(service.codeSender ? { code_sender: service.codeSender } : {}),
      },
    ]
  }
  if (call === 'POST /v1/auth/codes') {
    if (service.refuseCodes) return [429, { code: service.refuseCodes }]
    return [204, null]
  }
  if (call === 'POST /v1/auth/sessions') {
    const { code, identifier } = body as { code?: string; identifier?: string }
    if (code !== GOOD_CODE) return [400, { code: 'INVALID_CODE' }]
    // Anyone but Ana signs in for the first time, to an account with no name yet.
    service.account =
      identifier === ana.email ? ana : { ...ana, display_name: '', adult_confirmed: false }
    return [200, { account: service.account }]
  }
  // Like the service, nothing about an invitation is answered to someone
  // signed out.
  if (!service.account) return [401, { code: 'UNAUTHENTICATED' }]
  if (call === 'POST /v1/invitations/preview') {
    const offer = offerExchange()
    const { token } = body as { token?: string }
    return [
      200,
      {
        bound: false,
        display_code: token === OTHER_INVITATION ? OTHER_INVITATION_CODE : offer.display_code,
        expires_at: revision.expires_at,
        currency: offer.currency,
        timezone: offer.timezone,
        revision: offer.open_revision,
      },
    ]
  }
  if (call === 'POST /v1/invitations/report') return [204, null]
  if (call === 'POST /v1/invitations/claim') {
    const { only_if_yours: onlyIfYours } = body as { only_if_yours?: boolean }
    return onlyIfYours ? [404, { code: 'INVITATION_UNAVAILABLE' }] : [200, offerExchange()]
  }
  if (call === 'GET /v1/me') return [200, service.account]
  if (call === 'POST /v1/me/identifiers') {
    const { code, identifier } = body as { code: string; identifier: string }
    if (code !== GOOD_CODE) return [401, { code: 'INVALID_CODE' }]
    service.account = identifier.includes('@')
      ? { ...service.account, email: identifier }
      : { ...service.account, phone: identifier }
    return [200, service.account]
  }
  if (call === 'PATCH /v1/me') {
    service.account = { ...service.account, ...(body as Partial<Account>) }
    return [200, service.account]
  }
  if (call === 'GET /v1/exchanges') return [200, exchanges().map(summary)]
  if (call === 'GET /v1/blocks') return [200, []]
  // To anyone but a reviewer, every staff path is not found.
  if (call.includes(' /v1/staff/')) {
    if (service.account.id === staleRita.id) return [401, { code: 'SESSION_TOO_OLD' }]
    return (service.account.id === rita.id && staff(call)) || [404, { code: 'NOT_FOUND' }]
  }
  if (call === `POST /v1/exchanges/${DRAFT}/revisions`) {
    service.proposed = true
    return [200, { exchange: sentExchange(), invitation_token: SENT_INVITATION }]
  }
  if (service.proposed && call === `GET /v1/exchanges/${DRAFT}`) return [200, sentExchange()]
  for (const exchange of [...exchanges(), ...others()]) {
    const at = `/v1/exchanges/${exchange.id}`
    if (call === `GET ${at}`) return [200, exchange]
    if (call === `PUT ${at}/draft`) return [204, null]
    if (call === `GET ${at}/history`) return [200, history(exchange)]
    if (call === `GET ${at}/record`) return [200, record(exchange)]
    if (call === `GET ${at}/block`) return [200, { blocked: false, name: PARTIES.B }]
    if (call === `PUT ${at}/block`) return [204, null]
    if (call === `GET ${at}/sms-updates`) return [200, textUpdates(service, exchange)]
    if (call === `PUT ${at}/sms-updates`) {
      const { on } = body as { on: boolean }
      if (on && service.optedOut) return [409, { code: 'PHONE_OPTED_OUT' }]
      if (on) service.textUpdates.add(exchange.id)
      else service.textUpdates.delete(exchange.id)
      return [200, textUpdates(service, exchange)]
    }
  }
  return [404, { code: 'NOT_FOUND' }]
}
