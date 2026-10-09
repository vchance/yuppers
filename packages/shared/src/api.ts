import type {
  Account,
  ApiClient,
  Command,
  components,
  ErrorCode,
  ExchangeSummary,
  ExchangeView,
  Meta,
} from '@yuppers/api-client'

import { clientHeader, parseVersion, type ClientIdentity } from './client-version'
import { idempotencyKeys } from './idempotency'
import { invitationOptions } from './share'
import type { ReportReason } from './safety'

type Schemas = components['schemas']
export type BlockedPerson = Schemas['BlockedPerson']
export type BlockStatus = Schemas['BlockStatus']
export type CodeChannel = Schemas['CodeChannel']
export type DeletionPreview = Schemas['DeletionPreview']
export type DeviceRegistered = Schemas['DeviceRegistered']
export type RegisterDevice = Schemas['RegisterDevice']
export type InvitationPreview = Schemas['InvitationPreview']
export type RevisionSent = Schemas['RevisionSent']
export type RevisionView = Schemas['RevisionView']
export type SendRevision = Schemas['SendRevision']
export type SessionCreated = Schemas['SessionCreated']
export type Slot = Schemas['Slot']
export type WalletLink = Schemas['WalletLink']
export type SmsUpdates = Schemas['SmsUpdates']
export type SmsCodeConsent = Schemas['SmsCodeConsent']
export type SetSmsUpdates = Schemas['SetSmsUpdates']
export type WalletPlatform = Schemas['WalletPlatform']
export type PaymentHandles = Schemas['PaymentHandles']
export type PaymentHandleChanges = Schemas['PaymentHandleChanges']
export type PaymentOptionsShown = Schemas['PaymentOptionsShown']
export type PaymentOptionsView = Schemas['PaymentOptionsView']
export type CombineOffer = Schemas['CombineOffer']
export type BoundAddress = Schemas['BoundAddress']
export type IdentifierKind = Schemas['IdentifierKind']

/**
 * A refusal from the service, or no answer from it. Screens show
 * `wording.errors[code]`; nothing reads message text (DESIGN.md §13.3).
 */
export class ApiFailure extends Error {
  readonly code: ErrorCode
  /** No reply from the service itself, so whether the request took effect is unknown. */
  readonly unanswered: boolean
  /**
   * With `IDENTIFIER_ON_OTHER_ACCOUNT`: the offer to combine the other
   * account into this one (`identifiers.ts`).
   */
  readonly combine: CombineOffer | null

  constructor(code: ErrorCode, unanswered = false, combine: CombineOffer | null = null) {
    super(code)
    this.name = 'ApiFailure'
    this.code = code
    this.unanswered = unanswered
    this.combine = combine
  }
}

/** The offer to combine accounts a refusal carries, if it carries one. */
export function combineOffer(error: unknown): CombineOffer | null {
  return error instanceof ApiFailure ? error.combine : null
}

/** The error code to show for anything a request can throw. */
export function failureCode(error: unknown): ErrorCode {
  return error instanceof ApiFailure ? error.code : 'INTERNAL'
}

/**
 * How a client holds its session (DESIGN.md §8). A browser asks for a cookie
 * its page cannot read and never sees a token. An app asks for the token,
 * keeps it in the device's secure storage, and says here how to read it back.
 */
export type SessionHolding =
  | { delivery: 'COOKIE' }
  | { delivery: 'TOKEN'; token(): string | null }

export interface ExchangeApiOptions {
  client: ApiClient
  session: SessionHolding
  /** Makes an idempotency key. It must be unguessable; the default needs `crypto.randomUUID`. */
  newKey?: () => string
  /**
   * Which client this is and which build, named on every request so the
   * service can refuse a change from one too old (`CLIENT_TOO_OLD`).
   */
  identity?: ClientIdentity
}

interface Reply<T> {
  data?: T
  error?: unknown
  response: Response
}

export type ExchangeApi = ReturnType<typeof createExchangeApi>

/**
 * Every call the screens make, the same for the web app and the mobile app.
 * What differs between them is where the service is and how the session is
 * held, and both are given here.
 */
export function createExchangeApi({ client, session, newKey, identity }: ExchangeApiOptions) {
  const keys = idempotencyKeys(newKey)
  let signedOut: () => void = () => {}
  let tooOld: () => void = () => {}

  /** The session token, for a client that holds one. A cookie travels by itself. */
  function token(): string | null {
    return session.delivery === 'TOKEN' ? session.token() : null
  }

  function headers(): Record<string, string> | undefined {
    const sent: Record<string, string> = {}
    const held = token()
    if (held) sent.Authorization = `Bearer ${held}`
    // A build that cannot say which it is names none: the service would
    // ignore a version it cannot read anyway.
    if (identity && parseVersion(identity.version)) {
      sent['X-Client-Version'] = clientHeader(identity)
    }
    return Object.keys(sent).length > 0 ? sent : undefined
  }

  async function send<T>(request: () => Promise<Reply<T>>): Promise<T> {
    let reply: Reply<T>
    try {
      reply = await request()
    } catch {
      throw new ApiFailure('SERVICE_UNAVAILABLE', true)
    }
    if (reply.response.ok) return reply.data as T

    const body = reply.error
    const code =
      typeof body === 'object' &&
      body !== null &&
      typeof (body as { code?: unknown }).code === 'string'
        ? ((body as { code: string }).code as ErrorCode)
        : null
    // An error that is not the service's own came from something in between.
    if (code === null) throw new ApiFailure('SERVICE_UNAVAILABLE', true)
    if (code === 'UNAUTHENTICATED') signedOut()
    if (code === 'CLIENT_TOO_OLD') tooOld()
    const combine =
      code === 'IDENTIFIER_ON_OTHER_ACCOUNT'
        ? ((body as { combine?: CombineOffer }).combine ?? null)
        : null
    throw new ApiFailure(code, false, combine)
  }

  /**
   * Sends a change to an exchange with an idempotency key: a fresh one for
   * each attempt, the same one again when retrying a request that went
   * unanswered.
   */
  async function change<T>(
    path: string,
    body: unknown,
    request: (key: string) => Promise<Reply<T>>,
  ): Promise<T> {
    const fingerprint = `${path} ${JSON.stringify(body)}`
    const key = keys.keyFor(fingerprint)
    try {
      const result = await send(() => request(key))
      keys.answered(fingerprint)
      return result
    } catch (error) {
      if (error instanceof ApiFailure && error.unanswered) keys.unanswered(fingerprint, key)
      else keys.answered(fingerprint)
      throw error
    }
  }

  return {
    /** Registers what to do when the service says the session is no longer valid. */
    onSignedOut(handler: () => void): void {
      signedOut = handler
    },

    /** Registers what to do when the service says this build is too old to act. */
    onClientTooOld(handler: () => void): void {
      tooOld = handler
    },

    /** The service's identity, and how old a client may be. Needs no session. */
    meta(): Promise<Meta> {
      return send(() => client.GET('/v1/meta', { headers: headers() }))
    },

    /** The signed-in account, or `null` when nobody is signed in. */
    async me(): Promise<Account | null> {
      // Without a token there is no session to ask about.
      if (session.delivery === 'TOKEN' && token() === null) return null
      let reply: Reply<Account>
      try {
        reply = await client.GET('/v1/me', { headers: headers() })
      } catch {
        throw new ApiFailure('SERVICE_UNAVAILABLE', true)
      }
      if (reply.response.status === 401) return null
      return send(async () => reply)
    },

    /**
     * Sends a one-time code to an email address or phone number. For a
     * phone number, `smsConsent` says the box beside it was ticked
     * (`sms-code-consent.ts`); without it the service refuses.
     */
    requestCode(identifier: string, smsConsent?: SmsCodeConsent): Promise<void> {
      const body = smsConsent ? { identifier, sms_consent: smsConsent } : { identifier }
      return send(() => client.POST('/v1/auth/codes', { headers: headers(), body }))
    },

    /**
     * Signs in, creating the account the first time. For a token session the
     * reply carries the token, once; keeping it is the caller's job.
     */
    signIn(identifier: string, code: string, language: string): Promise<SessionCreated> {
      return send(() =>
        client.POST('/v1/auth/sessions', {
          headers: headers(),
          body: { identifier, code, delivery: session.delivery, language },
        }),
      )
    },

    signOut(): Promise<void> {
      return send(() => client.DELETE('/v1/auth/session', { headers: headers() }))
    },

    updateMe(update: Schemas['UpdateAccount']): Promise<Account> {
      return send(() => client.PATCH('/v1/me', { headers: headers(), body: update }))
    },

    // Deleting the account (DESIGN.md §4.1).

    /** What deleting the account would do to the exchanges it is in. Changes nothing. */
    deletionPreview(): Promise<DeletionPreview> {
      return send(() => client.GET('/v1/me/deletion', { headers: headers() }))
    },

    /**
     * Sends the code that confirms a deletion to the account's own email
     * address or phone number. The service knows the address; none is sent.
     * For the phone number, `smsConsent` says the box beside it was ticked.
     */
    requestDeletionCode(channel: CodeChannel, smsConsent?: SmsCodeConsent): Promise<void> {
      const body = smsConsent ? { channel, sms_consent: smsConsent } : { channel }
      return send(() => client.POST('/v1/me/deletion/codes', { headers: headers(), body }))
    },

    /**
     * Deletes the account, for good. Once this resolves every session has
     * ended, this one included; forgetting it on the device is the caller's job.
     */
    deleteAccount(channel: CodeChannel, code: string): Promise<void> {
      return send(() =>
        client.POST('/v1/me/deletion', { headers: headers(), body: { channel, code } }),
      )
    },

    // Push notifications (DESIGN.md §12), for an app on a device.

    /**
     * Registers this device's push token under the current session, or
     * brings it up to date. Signing out of the session removes it.
     */
    registerDevice(device: RegisterDevice): Promise<DeviceRegistered> {
      return send(() => client.PUT('/v1/me/devices', { headers: headers(), body: device }))
    },

    /** Stops push notifications to a device registered earlier. Fine to repeat. */
    removeDevice(id: string): Promise<void> {
      return send(() =>
        client.DELETE('/v1/me/devices/{id}', { headers: headers(), params: { path: { id } } }),
      )
    },

    listExchanges(): Promise<ExchangeSummary[]> {
      return send(() => client.GET('/v1/exchanges', { headers: headers() }))
    },

    createExchange(timezone: string): Promise<ExchangeView> {
      return send(() => client.POST('/v1/exchanges', { headers: headers(), body: { timezone } }))
    },

    getExchange(id: string): Promise<ExchangeView> {
      return send(() =>
        client.GET('/v1/exchanges/{id}', { headers: headers(), params: { path: { id } } }),
      )
    },

    /**
     * The latest of an exchange's history, with what the parties wrote along
     * the way; or, given the `earlier` of a page, the page before it.
     */
    history(id: string, before?: number | null): Promise<Schemas['HistoryPage']> {
      return send(() =>
        client.GET('/v1/exchanges/{id}/history', {
          headers: headers(),
          params: { path: { id }, query: before == null ? {} : { before } },
        }),
      )
    },

    /** One part of an exchange's record: the first, or the one starting at `from`. */
    recordPart(
      id: string,
      from: Schemas['Continuation'] | null,
    ): Promise<Schemas['RecordDocument']> {
      return send(() =>
        client.GET('/v1/exchanges/{id}/record', {
          headers: headers(),
          params: { path: { id }, query: from ?? {} },
        }),
      )
    },

    /** Saves the working copy. It is private to its author and binds nobody. */
    saveDraft(id: string, draft: object): Promise<void> {
      return send(() =>
        client.PUT('/v1/exchanges/{id}/draft', {
          headers: headers(),
          params: { path: { id } },
          // The service stores it as given; the generated type cannot say so.
          body: { body: draft as Record<string, never> },
        }),
      )
    },

    sendRevision(id: string, body: SendRevision): Promise<RevisionSent> {
      return change(`revisions/${id}`, body, (key) =>
        client.POST('/v1/exchanges/{id}/revisions', {
          headers: headers(),
          params: { path: { id }, header: { 'Idempotency-Key': key } },
          body,
        }),
      )
    },

    /** Every change names the version it was based on (DESIGN.md §13.4). */
    runCommand(id: string, expectedVersion: number, command: Command): Promise<ExchangeView> {
      const body = { expected_version: expectedVersion, command }
      return change(`commands/${id}`, body, (key) =>
        client.POST('/v1/exchanges/{id}/commands', {
          headers: headers(),
          params: { path: { id }, header: { 'Idempotency-Key': key } },
          body,
        }),
      )
    },

    /**
     * Gives up the invited party's place, for someone the initiator has not
     * confirmed (DESIGN.md §8). Afterwards the exchange no longer exists for
     * them, so there is nothing to answer with.
     */
    leaveExchange(id: string): Promise<void> {
      return send(() =>
        client.POST('/v1/exchanges/{id}/leave', { headers: headers(), params: { path: { id } } }),
      )
    },

    /** Replaces the invitation link and returns the new token, which is shown once. */
    async reissueInvitation(id: string, boundTo: string | null): Promise<string> {
      const issued = await send(() =>
        client.POST('/v1/exchanges/{id}/invitation', {
          headers: headers(),
          params: { path: { id } },
          body: invitationOptions(boundTo),
        }),
      )
      return issued.invitation_token
    },

    // The invitation token travels in the body, so it never appears in a URL
    // the service might log.
    previewInvitation(invitation: string): Promise<InvitationPreview> {
      return send(() =>
        client.POST('/v1/invitations/preview', { headers: headers(), body: { token: invitation } }),
      )
    },

    /** Takes the invited party's place. Only ever on the person's own say-so. */
    claimInvitation(invitation: string): Promise<ExchangeView> {
      return send(() =>
        client.POST('/v1/invitations/claim', { headers: headers(), body: { token: invitation } }),
      )
    },

    /**
     * The exchange behind a link whose place this account already took, for
     * taking the person back to it. Never takes a place: for anyone else,
     * and at a link still live, it is refused with `INVITATION_UNAVAILABLE`.
     */
    invitationAlreadyYours(invitation: string): Promise<ExchangeView> {
      return send(() =>
        client.POST('/v1/invitations/claim', {
          headers: headers(),
          body: { token: invitation, only_if_yours: true },
        }),
      )
    },

    // Reporting and blocking (DESIGN.md §9).

    /** Reports an exchange, and with it the other party. Repeating it is harmless. */
    reportExchange(id: string, reason: ReportReason, details: string | null): Promise<void> {
      return send(() =>
        client.POST('/v1/exchanges/{id}/reports', {
          headers: headers(),
          params: { path: { id } },
          body: { reason, details },
        }),
      )
    },

    // Like the preview, the invitation token travels in the body, never in a URL.
    reportInvitation(
      invitation: string,
      reason: ReportReason,
      details: string | null,
    ): Promise<void> {
      return send(() =>
        client.POST('/v1/invitations/report', {
          headers: headers(),
          body: { token: invitation, reason, details },
        }),
      )
    },

    /** Whether the person signed in has blocked the other party of this exchange, and that party's name. */
    blockStatus(id: string): Promise<BlockStatus> {
      return send(() =>
        client.GET('/v1/exchanges/{id}/block', { headers: headers(), params: { path: { id } } }),
      )
    },

    block(id: string): Promise<void> {
      return send(() =>
        client.PUT('/v1/exchanges/{id}/block', { headers: headers(), params: { path: { id } } }),
      )
    },

    unblock(id: string): Promise<void> {
      return send(() =>
        client.DELETE('/v1/exchanges/{id}/block', {
          headers: headers(),
          params: { path: { id } },
        }),
      )
    },

    blockedPeople(): Promise<BlockedPerson[]> {
      return send(() => client.GET('/v1/blocks', { headers: headers() }))
    },

    // Wallet passes (DESIGN.md §11). Each is the caller's own pass for the
    // exchange, the same one each time; asking again is harmless.

    /**
     * A link that downloads the caller's Apple Wallet pass for a few minutes,
     * without a session: opened in Safari, it adds the pass to Wallet.
     */
    appleWalletLink(id: string): Promise<WalletLink> {
      return send(() =>
        client.POST('/v1/exchanges/{id}/wallet/apple/link', {
          headers: headers(),
          params: { path: { id } },
        }),
      )
    },

    /**
     * Attaches a verified email address or phone number to the account, with
     * the code sent to it (`requestCode`), or replaces the one of its kind.
     */
    addIdentifier(identifier: string, code: string): Promise<Account> {
      return send(() =>
        client.POST('/v1/me/identifiers', { headers: headers(), body: { identifier, code } }),
      )
    },

    /**
     * Removes the account's email address or phone number, with a code sent
     * to the other one, which stays (`requestCode`). Refused for the only one.
     */
    removeIdentifier(kind: 'email' | 'phone', code: string): Promise<Account> {
      const body = { code }
      return change(`identifiers/${kind}`, body, (key) =>
        client.DELETE('/v1/me/identifiers/{kind}', {
          headers: headers(),
          params: { path: { kind }, header: { 'Idempotency-Key': key } },
          body,
        }),
      )
    },

    /** Combines into this account the one an offer names. It cannot be undone. */
    combineAccounts(token: string): Promise<Account> {
      const body = { token }
      return change('combine', body, (key) =>
        client.POST('/v1/me/combine', {
          headers: headers(),
          params: { header: { 'Idempotency-Key': key } },
          body,
        }),
      )
    },

    /**
     * Sends a code to the address an invitation names (`sent_to`), which the
     * client never sees in full. For a number, `smsConsent` says the box was ticked.
     */
    requestInvitationAddressCode(invitation: string, smsConsent?: SmsCodeConsent): Promise<void> {
      const body = smsConsent
        ? { token: invitation, sms_consent: smsConsent }
        : { token: invitation }
      return send(() =>
        client.POST('/v1/invitations/address/codes', { headers: headers(), body }),
      )
    },

    /**
     * Adds the address an invitation names to the account, with the code sent
     * there, and opens the invitation. `replace` for an account that has
     * another of that kind.
     */
    addInvitationAddress(invitation: string, code: string, replace: boolean): Promise<ExchangeView> {
      return send(() =>
        client.POST('/v1/invitations/address', {
          headers: headers(),
          body: { token: invitation, code, replace },
        }),
      )
    },

    // Text updates for an agreement (DESIGN.md §12): "Yuppers.app agreement updates".

    /** Where the signed-in party stands on text updates for an agreement. */
    smsUpdates(id: string): Promise<SmsUpdates> {
      return send(() =>
        client.GET('/v1/exchanges/{id}/sms-updates', {
          headers: headers(),
          params: { path: { id } },
        }),
      )
    },

    /**
     * Turns text updates for an agreement on, with the version and language
     * of the consent wording shown beside the box, or off.
     */
    setSmsUpdates(id: string, body: SetSmsUpdates): Promise<SmsUpdates> {
      return send(() =>
        client.PUT('/v1/exchanges/{id}/sms-updates', {
          headers: headers(),
          params: { path: { id } },
          body,
        }),
      )
    },

    // Payment options (`payments.ts`). Yuppers never moves money.

    /** The account's own payment options; each one not saved is null. */
    paymentHandles(): Promise<PaymentHandles> {
      return send(() => client.GET('/v1/me/payment-handles', { headers: headers() }))
    },

    /** Saves the account's payment options, replacing what was saved: one left null is removed. */
    setPaymentHandles(handles: PaymentHandles): Promise<PaymentHandles> {
      return send(() =>
        client.PUT('/v1/me/payment-handles', { headers: headers(), body: handles }),
      )
    },

    /** Removes all of the account's payment options, and stops showing them on every yup. */
    removePaymentHandles(): Promise<void> {
      return send(() => client.DELETE('/v1/me/payment-handles', { headers: headers() }))
    },

    /** Shows the account's payment options to the other party of a yup, or stops. */
    setPaymentOptions(id: string, on: boolean): Promise<PaymentOptionsShown> {
      return send(() =>
        client.PUT('/v1/exchanges/{id}/payment-options', {
          headers: headers(),
          params: { path: { id } },
          body: { on },
        }),
      )
    },

    /** The "Save to Google Wallet" link for the caller's pass. */
    googleWalletLink(id: string): Promise<WalletLink> {
      return send(() =>
        client.POST('/v1/exchanges/{id}/wallet/google', {
          headers: headers(),
          params: { path: { id } },
        }),
      )
    },
  }
}
