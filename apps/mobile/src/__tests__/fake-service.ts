import type { Account, ErrorCode, ExchangeView } from '@yuppers/api-client';
import type { PaymentHandles, RevisionView, Wording } from '@yuppers/shared';
import { seriesInExchange, TERMS_VERSION } from '@yuppers/shared';
import { fireEvent, screen } from '@testing-library/react-native';

import { answerRecordAndSafety } from './fake-record';

/*
 * A stand-in for the service, for running the whole app in a test: enough of
 * the API to sign in and work an exchange, with every request kept so a test
 * can say what the app sent.
 */

export const EXCHANGE = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70';
export const DRAFT = '5d0c3b1a-2f64-4e8b-9a7d-1c2e3f4a5b6c';
export const REPAIR = '11111111-1111-4111-8111-111111111111';
export const PAYMENT = '22222222-2222-4222-8222-222222222222';
export const TOKEN = 'session-token-from-the-service';
export const INVITATION = 'a3'.repeat(32);
/** The token of the invitation the draft's first proposal gets when it is sent. */
export const SENT_INVITATION = 'c5'.repeat(32);
/** The address the stand-in says codes come from, unless a test says otherwise. */
export const CODE_SENDER = 'codes@yuppers.example';
/** The ID the service gives a device registered for push. */
export const DEVICE = 'd0000000-0000-4000-8000-000000000001';

export const ana: Account = {
  id: 'a0000000-0000-4000-8000-000000000001',
  display_name: 'Ana Ruiz',
  adult_confirmed: true,
  language: 'en',
  email: 'ana@example.test',
};

const revision: RevisionView = {
  id: 'c0000000-0000-4000-8000-000000000001',
  sequence: 1,
  author: 'A',
  accepted_by: ['A', 'B'],
  content_hash: 'ab'.repeat(32),
  expires_at: '2026-10-16T12:00:00Z',
  terms: {
    party_a_name: 'Ana Ruiz',
    party_b_name: 'Ben Ortiz',
    terms: 'Repair the back fence.',
    contributions: [
      {
        id: REPAIR,
        from: 'A',
        type: 'SERVICE',
        description: 'Repair the back fence',
        due: { kind: 'DATE', date: '2026-10-30' },
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
};

function activeExchange(): ExchangeView {
  return {
    id: EXCHANGE,
    version: 7,
    state: 'ACTIVE',
    you: 'A',
    counterparty: 'CONFIRMED',
    currency: 'USD',
    timezone: 'America/Chicago',
    display_code: 'PVVS-5Q2K',
    in_force_revision: revision,
    contributions: [
      { id: REPAIR, status: 'PENDING' },
      { id: PAYMENT, status: 'PENDING' },
    ],
  };
}

function draftExchange(): ExchangeView {
  return {
    id: DRAFT,
    version: 1,
    state: 'DRAFT',
    you: 'A',
    counterparty: 'UNCLAIMED',
    currency: 'USD',
    timezone: 'America/Chicago',
    display_code: 'DRFT-0001',
    contributions: [],
    draft: {
      format: 1,
      base: null,
      partyA: 'Ana Ruiz',
      partyB: 'Ben Ortiz',
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
      ],
    } as never,
  };
}

/**
 * The draft once its first proposal is sent: open, with nobody invited yet,
 * and the link shared once the app has said so.
 */
function sentExchange(service: FakeService): ExchangeView {
  const { draft: _draft, ...sent } = draftExchange();
  return {
    ...sent,
    version: 2,
    state: 'NEGOTIATING',
    counterparty: 'UNCLAIMED',
    open_revision: { ...revision, accepted_by: ['A'] },
    invitation_open: true,
    invitation_shared_at: service.sharedAt,
  };
}

/**
 * A first proposal sent a while ago, by the reader, whose link nobody has
 * joined through: shared when the stand-in says so, and never otherwise.
 * Put in `service.exchange` by a test that wants the reminder.
 */
export function waitingExchange(sharedAt: string | null): ExchangeView {
  return {
    ...activeExchange(),
    version: 2,
    state: 'NEGOTIATING',
    counterparty: 'UNCLAIMED',
    in_force_revision: undefined,
    open_revision: { ...revision, accepted_by: ['A'] },
    contributions: [],
    invitation_open: true,
    invitation_shared_at: sharedAt,
  };
}

export const INSTALMENT_ONE = '33333333-3333-4333-8333-333333333331';
export const INSTALMENT_TWO = '33333333-3333-4333-8333-333333333332';
export const INSTALMENT_THREE = '33333333-3333-4333-8333-333333333333';

/**
 * An agreement in which the reader owes three payments, the first confirmed,
 * and provides one job: for series counts, "Mark the rest as paid" and
 * progress notes. Put in `service.exchange` by a test.
 */
export function seriesExchange(): ExchangeView {
  const money = (id: string, number: number) => ({
    id,
    from: 'A' as const,
    type: 'MONEY' as const,
    description: `Repayment ${number} of 3`,
    due: { kind: 'DATE' as const, date: `2026-1${number}-15` },
    required: true,
    amount_minor: 3333,
  });
  const base = activeExchange();
  return {
    ...base,
    in_force_revision: {
      ...revision,
      terms: {
        ...revision.terms,
        contributions: [
          revision.terms.contributions[0],
          money(INSTALMENT_ONE, 1),
          money(INSTALMENT_TWO, 2),
          money(INSTALMENT_THREE, 3),
        ],
      },
    },
    contributions: [
      { id: REPAIR, status: 'PENDING' },
      { id: INSTALMENT_ONE, status: 'ACCEPTED' },
      { id: INSTALMENT_TWO, status: 'PENDING' },
      { id: INSTALMENT_THREE, status: 'PENDING' },
    ],
  };
}

/** A progress note the stand-in accepted. */
export interface ProgressNoted {
  contribution: string;
  note: string;
}

export interface Sent {
  method: string;
  path: string;
  authorization: string | null;
  idempotencyKey: string | null;
  /** How the app named itself and its build. */
  clientVersion: string | null;
  body: unknown;
}

export interface FakeService {
  sent: Sent[];
  /** The account behind the session token, once there is one. */
  account: Account | null;
  exchange: ExchangeView;
  /** The other party's name in the list of exchanges. */
  otherPartyName: string;
  /** Makes the next command fail as if the other party had acted first. */
  conflictNext: boolean;
  /**
   * The invitation link `INVITATION`: still open, used by someone else, or
   * used by the signed-in account.
   */
  invitation: 'live' | 'spent' | 'yours';
  /** Whether the service says it sends push notifications. */
  push: boolean;
  /** Whether the service says it can send codes to phone numbers. */
  phone: boolean;
  /** The address the service says codes come from, if it says. */
  codeSender: string | null;
  /** A refusal for every request for a code from now on, such as a limit. */
  refuseCodes: ErrorCode | null;
  /** Whether the draft's first proposal has been sent. */
  proposed: boolean;
  /** When the app said the sent proposal's link was shared, once it has. */
  sharedAt: string | null;
  /** The list of exchanges as someone new sees it: empty. */
  noExchanges: boolean;
  /** The body of the request that made a draft, if one was made. */
  created: unknown;
  /** The working copy last saved into the draft that was made, if any. */
  saved: unknown;
  /** The devices registered for push, by ID, with what was registered. */
  devices: Map<string, unknown>;
  /** Whether the service texts agreement updates (`sms_updates` in its meta). */
  texting: boolean;
  /** The agreements the account has turned text updates on for. */
  textUpdates: Set<string>;
  /** Whether the account's number replied STOP. */
  optedOut: boolean;
  /** The account's own payment options. */
  handles: PaymentHandles;
  /** The progress notes accepted so far, which the history then shows. */
  progress: ProgressNoted[];
  /** Makes the next progress note fail as if the item held as many as it can. */
  progressFull: boolean;
  fetch: typeof fetch;
}

export function fakeService(): FakeService {
  const service: FakeService = {
    sent: [],
    account: null,
    exchange: activeExchange(),
    otherPartyName: 'Ben Ortiz',
    conflictNext: false,
    invitation: 'live',
    push: false,
    phone: true,
    codeSender: CODE_SENDER,
    refuseCodes: null,
    proposed: false,
    sharedAt: null,
    noExchanges: false,
    created: null,
    saved: null,
    devices: new Map(),
    texting: true,
    textUpdates: new Set(),
    optedOut: false,
    progress: [],
    progressFull: false,
    handles: { venmo: null, cash_app: null, paypal: null, zelle: null },
    fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
      const request = input instanceof Request ? input : new Request(input, init);
      const text = await request.text();
      const body = text ? JSON.parse(text) : null;
      const path = new URL(request.url).pathname;
      const authorization = request.headers.get('Authorization');
      service.sent.push({
        method: request.method,
        path,
        authorization,
        idempotencyKey: request.headers.get('Idempotency-Key'),
        clientVersion: request.headers.get('X-Client-Version'),
        body,
      });
      const [status, answer] = respond(service, request.method, path, authorization, body);
      return new Response(answer === null ? null : JSON.stringify(answer), {
        status,
        headers: answer === null ? {} : { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch,
  };
  return service;
}

function respond(
  service: FakeService,
  method: string,
  path: string,
  authorization: string | null,
  body: unknown,
): [number, unknown] {
  const call = `${method} ${path}`;
  if (call === 'GET /v1/meta') {
    return [
      200,
      {
        service: 'yuppers-backend',
        version: '0.0.0',
        commit: 'unknown',
        built_at: null,
        minimum_client_versions: { web: null, ios: null, android: null },
        push_notifications: service.push,
        // Both, so a device's own wallet button shows on an agreement in force.
        wallet_platforms: ['APPLE', 'GOOGLE'],
        sign_in_channels: service.phone ? ['email', 'phone'] : ['email'],
        sms_country_codes: service.phone ? ['+1'] : [],
        sms_updates: service.texting,
        ...(service.codeSender ? { code_sender: service.codeSender } : {}),
      },
    ];
  }
  if (call === 'POST /v1/auth/codes') {
    // Like the service: a code by text only with the box beside the number ticked.
    const { identifier, sms_consent } = body as { identifier: string; sms_consent?: unknown };
    if (!identifier.includes('@') && !sms_consent) return [422, { code: 'SMS_CONSENT_REQUIRED' }];
    if (service.refuseCodes) return [429, { code: service.refuseCodes }];
    return [204, null];
  }
  if (call === 'POST /v1/auth/sessions') {
    // Like the service: the version of the sentence shown above the button.
    if ((body as { terms_version?: string }).terms_version !== TERMS_VERSION) {
      return [422, { code: 'TERMS_VERSION_UNKNOWN' }];
    }
    service.account = { ...ana, display_name: '', adult_confirmed: false };
    return [200, { account: service.account, token: TOKEN }];
  }
  // History, the record, reporting and blocking are answered in `fake-record.ts`.
  const signedIn = authorization === `Bearer ${TOKEN}` && service.account !== null;
  const answered = answerRecordAndSafety(service, call, signedIn);
  if (answered) return answered;

  // Everything else needs the session, the invitation's proposal included:
  // nothing about a link is answered to someone signed out.
  if (!signedIn || !service.account) return [401, { code: 'UNAUTHENTICATED' }];
  if (call === 'POST /v1/invitations/preview') {
    // Every way a link can be dead looks the same, used by this account or not.
    if (service.invitation !== 'live') return [404, { code: 'INVITATION_UNAVAILABLE' }];
    return [
      200,
      {
        bound: false,
        display_code: service.exchange.display_code,
        expires_at: revision.expires_at,
        currency: service.exchange.currency,
        timezone: service.exchange.timezone,
        revision: { ...revision, accepted_by: ['A'] },
      },
    ];
  }

  if (call === 'GET /v1/me') return [200, service.account];
  if (call === 'GET /v1/me/payment-handles') return [200, service.handles];
  const oneHandle = /^(PUT|DELETE) \/v1\/me\/payment-handles\/(venmo|cash_app|paypal|zelle)$/.exec(call);
  if (oneHandle) {
    const kind = oneHandle[2] as keyof PaymentHandles;
    const value = oneHandle[1] === 'PUT' ? (body as { value: string }).value : null;
    service.handles = { ...service.handles, [kind]: value };
    return [200, service.handles];
  }
  if (call === 'POST /v1/me/identifiers') {
    const { code, identifier } = body as { code: string; identifier: string };
    if (code !== '123456') return [401, { code: 'INVALID_CODE' }];
    service.account = identifier.includes('@')
      ? { ...service.account, email: identifier }
      : { ...service.account, phone: identifier };
    return [200, service.account];
  }
  for (const exchange of [service.exchange]) {
    const at = `/v1/exchanges/${exchange.id}/sms-updates`;
    const standing = () => ({
      on: service.textUpdates.has(exchange.id),
      available: service.texting && ['NEGOTIATING', 'ACTIVE'].includes(exchange.state),
      phone: service.account?.phone ?? null,
      opted_out: service.optedOut,
      consent_version: '2026-10-05',
    });
    if (call === `GET ${at}`) return [200, standing()];
    if (call === `PUT ${at}`) {
      const { on } = body as { on: boolean };
      if (on && service.optedOut) return [409, { code: 'PHONE_OPTED_OUT' }];
      if (on) service.textUpdates.add(exchange.id);
      else service.textUpdates.delete(exchange.id);
      return [200, standing()];
    }
  }
  if (call === 'POST /v1/invitations/claim') {
    const { only_if_yours } = body as { token: string; only_if_yours?: boolean };
    if (service.invitation === 'yours') return [200, service.exchange];
    if (service.invitation === 'live' && !only_if_yours) {
      service.invitation = 'yours';
      return [200, service.exchange];
    }
    return [404, { code: 'INVITATION_UNAVAILABLE' }];
  }
  if (call === 'PATCH /v1/me') {
    const update = body as { display_name?: string; adult_confirmed?: boolean; language?: string };
    service.account = {
      ...service.account,
      display_name: update.display_name ?? service.account.display_name,
      adult_confirmed: update.adult_confirmed ?? service.account.adult_confirmed,
      language: update.language ?? service.account.language,
    };
    return [200, service.account];
  }
  if (call === 'DELETE /v1/auth/session') {
    // The device signed in with the session goes with it.
    service.devices.clear();
    return [204, null];
  }
  if (call === 'PUT /v1/me/devices') {
    service.devices.set(DEVICE, body);
    return [200, { id: DEVICE }];
  }
  if (call === `DELETE /v1/me/devices/${DEVICE}`) {
    service.devices.delete(DEVICE);
    return [204, null];
  }
  if (call === 'GET /v1/exchanges') {
    if (service.noExchanges) return [200, []];
    return [
      200,
      [
        {
          id: service.exchange.id,
          display_code: service.exchange.display_code,
          other_party_name: service.otherPartyName,
          state: service.exchange.state,
          updated_at: '2026-10-02T06:30:00Z',
          you: 'A',
          counterparty: service.exchange.counterparty,
          invitation_shared_at: service.exchange.invitation_shared_at ?? null,
          ...counts(service.exchange),
        },
      ],
    ];
  }
  if (call === `GET /v1/exchanges/${EXCHANGE}`) return [200, service.exchange];
  if (call === `POST /v1/exchanges/${DRAFT}/revisions`) {
    service.proposed = true;
    return [200, { exchange: sentExchange(service), invitation_token: SENT_INVITATION }];
  }
  // Starting a yup: the draft that is made starts empty, and is read back
  // with whatever working copy was saved into it.
  if (call === 'POST /v1/exchanges') {
    service.created = body;
    service.saved = null;
    return [200, { ...draftExchange(), draft: undefined }];
  }
  if (call === `GET /v1/exchanges/${DRAFT}`) {
    if (service.proposed) return [200, sentExchange(service)];
    if (service.created) return [200, { ...draftExchange(), draft: service.saved ?? undefined }];
    return [200, draftExchange()];
  }
  // The initiator opened a way to send the link: the latest time is kept.
  if (call === `POST /v1/exchanges/${DRAFT}/invitation/shared`) {
    service.sharedAt = '2026-10-02T06:35:00Z';
    return [204, null];
  }
  if (call === `POST /v1/exchanges/${EXCHANGE}/invitation/shared`) {
    service.exchange = { ...service.exchange, invitation_shared_at: '2026-10-02T06:35:00Z' };
    return [204, null];
  }
  if (call === `PUT /v1/exchanges/${DRAFT}/draft`) {
    service.saved = (body as { body: unknown }).body;
    return [204, null];
  }
  if (call === `POST /v1/exchanges/${EXCHANGE}/commands`) {
    const { expected_version, command } = body as {
      expected_version: number;
      command: { type: string; contribution?: string; action?: string };
    };
    if (service.conflictNext || expected_version !== service.exchange.version) {
      // The other party got there first.
      service.conflictNext = false;
      service.exchange = {
        ...service.exchange,
        version: service.exchange.version + 1,
        end_proposed_by: 'B',
      };
      return [409, { code: 'VERSION_CONFLICT' }];
    }
    if (command.type === 'CONTRIBUTION' && command.action === 'CLAIM') {
      service.exchange = {
        ...service.exchange,
        version: service.exchange.version + 1,
        contributions: service.exchange.contributions.map((item) =>
          item.id === command.contribution ? { ...item, status: 'CLAIMED' } : item,
        ),
      };
      return [200, service.exchange];
    }
    const rest = command as { type: string; contributions?: string[]; note?: string };
    if (rest.type === 'CLAIM_REST' && rest.contributions) {
      const ids = rest.contributions;
      service.exchange = {
        ...service.exchange,
        version: service.exchange.version + 1,
        contributions: service.exchange.contributions.map((item) =>
          ids.includes(item.id) ? { ...item, status: 'CLAIMED' } : item,
        ),
      };
      return [200, service.exchange];
    }
    if (rest.type === 'NOTE_PROGRESS' && command.contribution) {
      const item = service.exchange.in_force_revision?.terms.contributions.find(
        (candidate) => candidate.id === command.contribution,
      );
      // Only the one who provides the item can write about its progress.
      if (!item || item.from !== service.exchange.you) return [403, { code: 'WRONG_ACTOR' }];
      if (service.progressFull) return [429, { code: 'TOO_MANY_REQUESTS' }];
      service.progress.push({ contribution: item.id, note: rest.note ?? '' });
      service.exchange = { ...service.exchange, version: service.exchange.version + 1 };
      return [200, service.exchange];
    }
    if (command.type === 'REQUEST_CLOSE' && !service.exchange.close_requested_by) {
      service.exchange = {
        ...service.exchange,
        version: service.exchange.version + 1,
        close_requested_by: 'A',
        close_requested_at: '2026-10-03T09:00:00Z',
        close_request_lapses_at: '2026-10-10T09:00:00Z',
      };
      return [200, service.exchange];
    }
    return [409, { code: 'ACTION_NOT_ALLOWED' }];
  }
  return [404, { code: 'NOT_FOUND' }];
}

/**
 * Signs in from the form on the screen, as someone new: the service's stand-in
 * takes any code and makes an account with no name yet.
 */
export async function signInOnScreen(w: Wording): Promise<void> {
  await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), 'ben@example.test');
  await fireEvent.press(screen.getByRole('button', { name: w.signIn.sendCode }));
  await fireEvent.changeText(await screen.findByLabelText(w.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.signIn.submit }));
}

/** The counts the list carries for a series of payments or stages. */
function counts(exchange: ExchangeView) {
  const found: Record<string, { total: number; confirmed: number; disputed: number }> = {};
  for (const series of seriesInExchange(exchange)) {
    const sum = found[series.kind] ?? { total: 0, confirmed: 0, disputed: 0 };
    found[series.kind] = {
      total: sum.total + series.total,
      confirmed: sum.confirmed + series.confirmed,
      disputed: sum.disputed + series.disputed,
    };
  }
  return found;
}
