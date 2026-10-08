import { randomUUID } from 'node:crypto'

import { codeFrom } from './codes'
import { apiURL, webURL } from './env'

/*
 * Someone acting through the API rather than the app: in most tests, the
 * person who starts the exchange, while the one under test uses the app.
 * The calls are the ones the clients make (scripts/load-check.mjs takes the
 * same path), straight to the service with a bearer token.
 */

// Must match the API's current consent wording version (backend/src/bin/api.rs).
const CONSENT = { language: 'en', version: 'draft-1' }

export type Kind = 'ITEM' | 'SERVICE' | 'TASK' | 'MONEY' | 'OTHER'

export interface Contribution {
  /** Who provides it: the initiator (A) or the person invited (B). */
  from: 'A' | 'B'
  kind: Kind
  description: string
  /** For money, in minor units of the exchange's currency. */
  amountMinor?: number
}

export interface Exchange {
  id: string
  version: number
  state: string
  [key: string]: unknown
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string | undefined,
    message: string,
  ) {
    super(message)
  }
}

export class ApiPerson {
  private constructor(
    readonly name: string,
    readonly email: string,
    readonly token: string,
  ) {}

  /**
   * Signs in with a code from the API's log and sets up the profile: a new
   * account, or a second session for one made in the app.
   */
  static async signUp(name: string, email: string, language = 'en'): Promise<ApiPerson> {
    const code = await codeFrom(email, 'sign-in', () =>
      call('POST', '/v1/auth/codes', { body: { identifier: email } }),
    )
    const session = (await call('POST', '/v1/auth/sessions', {
      body: { identifier: email, code, delivery: 'TOKEN', language },
    })) as { token: string }
    const person = new ApiPerson(name, email, session.token)
    await person.call('PATCH', '/v1/me', { display_name: name, adult_confirmed: true })
    return person
  }

  call(method: string, path: string, body?: unknown, idempotent = false): Promise<unknown> {
    return call(method, path, { body, token: this.token, idempotent })
  }

  view(id: string): Promise<Exchange> {
    return this.call('GET', `/v1/exchanges/${id}`) as Promise<Exchange>
  }

  /**
   * Runs a command at the version of the exchange as it stands now. If the
   * other party changed it in between, reads it again and retries, as a
   * client does once it has shown the person the change.
   */
  async command(id: string, command: Record<string, unknown>): Promise<Exchange> {
    for (let attempt = 1; ; attempt += 1) {
      const seen = await this.view(id)
      try {
        return (await this.call(
          'POST',
          `/v1/exchanges/${id}/commands`,
          { expected_version: seen.version, command },
          true,
        )) as Exchange
      } catch (error) {
        if (!(error instanceof ApiError && error.code === 'VERSION_CONFLICT') || attempt >= 5) {
          throw error
        }
      }
    }
  }

  /**
   * A first proposal to someone named `otherName`, from a new exchange to
   * the invitation link, as the app would show it to the person sending it.
   * Returns the ids given to the contributions too, in order.
   */
  async propose(
    otherName: string,
    contributions: readonly Contribution[],
    options: { note?: string; language?: string } = {},
  ): Promise<{ id: string; link: string; contributionIds: string[] }> {
    const draft = (await this.call('POST', '/v1/exchanges', { timezone: 'UTC' })) as Exchange
    const contributionIds = contributions.map(() => randomUUID())
    const terms = {
      party_a_name: this.name,
      party_b_name: otherName,
      terms: '',
      contributions: contributions.map((item, index) => ({
        id: contributionIds[index],
        from: item.from,
        type: item.kind,
        description: item.description,
        quantity: null,
        due: { kind: 'ON_AGREEMENT' },
        completion_criteria: null,
        required: true,
        amount_minor: item.kind === 'MONEY' ? (item.amountMinor ?? 1000) : null,
      })),
    }
    const sent = (await this.call(
      'POST',
      `/v1/exchanges/${draft.id}/revisions`,
      {
        expected_version: draft.version,
        terms,
        consent: CONSENT,
        note: options.note ?? null,
        invitation: { for_anyone: true },
      },
      true,
    )) as { invitation_token: string }
    // The link the app hands its user to send, `{web origin}/{language}/i#{token}`
    // (README, "Mobile app"). Here the web origin is the harness, so the link
    // opens the app's own screens.
    const link = `${webURL}/${options.language ?? 'en'}/i#${sent.invitation_token}`
    return { id: draft.id, link, contributionIds }
  }

  /** A new exchange, left as a draft nobody else has seen. */
  async draft(): Promise<string> {
    const draft = (await this.call('POST', '/v1/exchanges', { timezone: 'UTC' })) as Exchange
    return draft.id
  }

  /** Throws away a draft, which closes it with nothing agreed. */
  discard(id: string): Promise<Exchange> {
    return this.command(id, { type: 'DISCARD' })
  }

  /** Takes back the proposal this person sent, which closes a first proposal. */
  async withdraw(id: string): Promise<Exchange> {
    const seen = (await this.view(id)) as Exchange & { open_revision: { id: string } }
    return this.command(id, { type: 'WITHDRAW', revision: seen.open_revision.id })
  }

  /** Says the person who opened the invitation is the one meant. */
  confirmCounterparty(id: string): Promise<Exchange> {
    return this.command(id, { type: 'CONFIRM_COUNTERPARTY' })
  }

  /** Records a contribution as delivered, or confirms the other's. */
  contribution(id: string, contribution: string, action: 'CLAIM' | 'CONFIRM'): Promise<Exchange> {
    return this.command(id, { type: 'CONTRIBUTION', contribution, action })
  }
}

async function call(
  method: string,
  path: string,
  options: { body?: unknown; token?: string; idempotent?: boolean },
): Promise<unknown> {
  const headers: Record<string, string> = {}
  if (options.token) headers.authorization = `Bearer ${options.token}`
  if (options.body !== undefined) headers['content-type'] = 'application/json'
  if (options.idempotent) headers['idempotency-key'] = randomUUID()
  const response = await fetch(apiURL + path, {
    method,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  })
  const text = await response.text()
  const json: unknown = text ? JSON.parse(text) : null
  if (!response.ok) {
    const code = (json as { code?: string } | null)?.code
    throw new ApiError(response.status, code, `${method} ${path}: ${response.status} ${code ?? ''}`)
  }
  return json
}
