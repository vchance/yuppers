import type { BlockedPerson, HistoryPage, RecordDocument, RecordEvent } from '@yuppers/shared';

import type { FakeService } from './fake-service';

/*
 * The part of the stand-in service that answers for an exchange's history
 * and record, and for reporting and blocking. `fake-service.ts` asks it
 * first; what it does not answer falls through to the rest.
 */

export const PARTIES = { A: 'Ana Ruiz', B: 'Ben Ortiz' };
const A_SIGNED = '2026-10-02T15:00:05Z';
const B_SIGNED = '2026-10-02T16:10:00Z';

export interface RecordAndSafety {
  /** Whether the person signed in has blocked the other party. */
  blocked: boolean;
  /** Makes reports fail as they do once a day's worth have been sent. */
  reportLimitReached: boolean;
  /** Makes the history unreadable, as when the service cannot be reached. */
  historyDown: boolean;
  /** A record to answer with instead of the one made from the exchange. */
  record: RecordDocument | null;
}

const states = new WeakMap<FakeService, RecordAndSafety>();

/** What a test can set about one stand-in service, and read back. */
export function recordAndSafety(service: FakeService): RecordAndSafety {
  let state = states.get(service);
  if (!state) {
    state = { blocked: false, reportLimitReached: false, historyDown: false, record: null };
    states.set(service, state);
  }
  return state;
}

/** Everything that has happened, oldest first, including what the test has done since. */
function events(service: FakeService): RecordEvent[] {
  const inForce = service.exchange.in_force_revision;
  if (!inForce) return [];
  const revision = { id: inForce.id, sequence: inForce.sequence };
  const happened: Omit<RecordEvent, 'sequence'>[] = [
    {
      type: 'REVISION_SENT',
      actor: 'A',
      at: A_SIGNED,
      revision,
      note: 'Here is what we talked about on Tuesday.',
    },
    { type: 'COUNTERPARTY_CLAIMED', actor: 'B', at: '2026-10-02T16:00:00Z' },
    { type: 'COUNTERPARTY_CONFIRMED', actor: 'A', at: '2026-10-02T16:05:00Z' },
    { type: 'REVISION_ACCEPTED', actor: 'B', at: B_SIGNED, revision },
    { type: 'AGREEMENT_IN_FORCE', actor: 'B', at: B_SIGNED, revision },
  ];
  // Each delivery the test has marked since, with the note written on it.
  for (const request of service.sent) {
    if (!request.path.endsWith('/commands')) continue;
    const { command } = request.body as {
      command: { type: string; action?: string; contribution?: string; note?: string | null };
    };
    if (command.type !== 'CONTRIBUTION' || command.action !== 'CLAIM') continue;
    const claimed = inForce.terms.contributions.find((item) => item.id === command.contribution);
    const stands = service.exchange.contributions.find((item) => item.id === command.contribution);
    if (!claimed || stands?.status !== 'CLAIMED') continue;
    happened.push({
      type: 'CONTRIBUTION_CLAIMED',
      actor: 'A',
      at: '2026-10-20T14:30:00Z',
      revision,
      contribution: { id: claimed.id, description: claimed.description },
      status: 'CLAIMED',
      note: command.note ?? null,
    });
  }
  // Each progress note the stand-in accepted, with the item it is on.
  for (const noted of service.progress) {
    const item = inForce.terms.contributions.find((candidate) => candidate.id === noted.contribution);
    if (!item) continue;
    happened.push({
      type: 'PROGRESS_NOTED',
      actor: item.from,
      at: '2026-10-21T08:00:00Z',
      revision,
      contribution: { id: item.id, description: item.description },
      note: noted.note,
    });
  }
  if (service.exchange.end_proposed_by) {
    happened.push({ type: 'END_PROPOSED', actor: 'B', at: '2026-10-21T09:00:00Z' });
  }
  return happened.map((event, index) => ({ ...event, sequence: index + 1 }));
}

function history(service: FakeService): HistoryPage {
  return { you: 'A', parties: PARTIES, earlier: null, events: events(service) };
}

/** The whole record in one document, as the service writes it for the initiator. */
export function recordOf(service: FakeService): RecordDocument {
  const { exchange } = service;
  const inForce = exchange.in_force_revision!;
  const happened = events(service);
  const verified = (method: 'EMAIL_OTP' | 'PHONE_OTP', at: string) => ({
    method,
    verified_at: at,
    description: 'described by the record',
  });
  return {
    format: 'exchange-record',
    format_version: 1,
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
      in_force_revision: { id: inForce.id, sequence: inForce.sequence },
      end_proposed_by: exchange.end_proposed_by ?? null,
      last_event: happened.length,
    },
    parties: PARTIES,
    contributions: inForce.terms.contributions.map((item) => {
      const stands = exchange.contributions.find((other) => other.id === item.id);
      return {
        id: item.id,
        from: item.from,
        description: item.description,
        required: item.required,
        status: stands?.status ?? 'PENDING',
        since: B_SIGNED,
      };
    }),
    revisions: [
      {
        id: inForce.id,
        sequence: inForce.sequence,
        author: 'A',
        sent_at: A_SIGNED,
        expires_at: inForce.expires_at,
        standing: { status: 'IN_FORCE', since: B_SIGNED, in_force_at: B_SIGNED },
        note: 'Here is what we talked about on Tuesday.',
        content_hash: inForce.content_hash,
        signed: {
          v: 1,
          exchange: exchange.id,
          currency: exchange.currency,
          timezone: exchange.timezone,
          parties: PARTIES,
          terms: inForce.terms.terms,
          attachments: [],
          contributions: inForce.terms.contributions.map((item) => ({
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
            name: PARTIES.A,
            signed_at: A_SIGNED,
            content_hash: inForce.content_hash,
            verification: verified('EMAIL_OTP', '2026-10-02T14:45:00Z'),
            consent: { language: 'en', version: 'draft-1' },
          },
          {
            party: 'B',
            name: PARTIES.B,
            signed_at: B_SIGNED,
            content_hash: inForce.content_hash,
            verification: verified('PHONE_OTP', '2026-10-02T15:58:00Z'),
            consent: { language: 'es', version: 'draft-1' },
          },
        ],
      },
    ],
    events: happened,
    part: { from: { revisions_after: 0, events_after: 0 }, next: null, complete: true },
  };
}

/** The other party, as the list of blocked people names them. */
export function blockedPerson(service: FakeService): BlockedPerson {
  return {
    exchange_id: service.exchange.id,
    display_code: service.exchange.display_code,
    name: PARTIES.B,
    blocked_at: '2026-10-22T18:30:00Z',
  };
}

/**
 * Answers a request about the record or about reporting and blocking, or
 * returns `null` for anything else. Everything here needs a session,
 * reporting a proposal from its invitation included.
 */
export function answerRecordAndSafety(
  service: FakeService,
  call: string,
  signedIn: boolean,
): [number, unknown] | null {
  const state = recordAndSafety(service);
  const exchange = `/v1/exchanges/${service.exchange.id}`;

  if (call === 'POST /v1/invitations/report') {
    if (!signedIn) return [401, { code: 'UNAUTHENTICATED' }];
    return state.reportLimitReached ? [429, { code: 'TOO_MANY_REQUESTS' }] : [204, null];
  }

  const mine = [
    `GET ${exchange}/history`,
    `GET ${exchange}/record`,
    `POST ${exchange}/reports`,
    `GET ${exchange}/block`,
    `PUT ${exchange}/block`,
    `DELETE ${exchange}/block`,
    'GET /v1/blocks',
  ];
  if (!mine.includes(call)) return null;
  if (!signedIn) return [401, { code: 'UNAUTHENTICATED' }];

  switch (call) {
    case `GET ${exchange}/history`:
      return state.historyDown ? [503, { code: 'SERVICE_UNAVAILABLE' }] : [200, history(service)];
    case `GET ${exchange}/record`:
      return [200, state.record ?? recordOf(service)];
    case `POST ${exchange}/reports`:
      return state.reportLimitReached ? [429, { code: 'TOO_MANY_REQUESTS' }] : [204, null];
    case `GET ${exchange}/block`:
      return [200, { blocked: state.blocked, name: PARTIES.B }];
    case `PUT ${exchange}/block`:
      state.blocked = true;
      return [204, null];
    case `DELETE ${exchange}/block`:
      state.blocked = false;
      return [204, null];
    default:
      return [200, state.blocked ? [blockedPerson(service)] : []];
  }
}
