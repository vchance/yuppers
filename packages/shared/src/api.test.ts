import type { ApiClient } from '@yuppers/api-client'
import { expect, test } from 'vitest'

import { ApiFailure, createExchangeApi, type SessionHolding } from './api'
import { TERMS_VERSION } from './terms'

const ID = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70'

interface Call {
  method: string
  path: string
  init: {
    headers?: Record<string, string>
    params?: { path?: Record<string, string>; header?: Record<string, string> }
    body?: unknown
  }
}

type Answer = { status: number; data?: unknown; error?: unknown } | 'no reply'

/** A stand-in for the generated client: records each call and answers from a script. */
function fakeClient(answers: Answer[] = []) {
  const calls: Call[] = []
  const call =
    (method: string) =>
    async (path: string, init: Call['init'] = {}) => {
      calls.push({ method, path, init })
      const answer = answers.shift() ?? { status: 200, data: {} }
      if (answer === 'no reply') throw new TypeError('Network request failed')
      const ok = answer.status >= 200 && answer.status < 300
      return { data: answer.data, error: answer.error, response: { ok, status: answer.status } }
    }
  const client = {
    GET: call('GET'),
    POST: call('POST'),
    PUT: call('PUT'),
    PATCH: call('PATCH'),
    DELETE: call('DELETE'),
  } as unknown as ApiClient
  return { client, calls }
}

function counted() {
  let next = 0
  return () => `key-${(next += 1)}`
}

const cookie: SessionHolding = { delivery: 'COOKIE' }
const holding = (token: string | null): SessionHolding => ({
  delivery: 'TOKEN',
  token: () => token,
})

test('a token session is sent as a bearer token on every call that needs one', async () => {
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: holding('s3cret'), newKey: counted() })
  await api.me()
  await api.listExchanges()
  await api.getExchange(ID)
  await api.runCommand(ID, 3, { type: 'PROPOSE_END' })
  await api.claimInvitation('a3'.repeat(32))
  expect(calls).toHaveLength(5)
  for (const made of calls) {
    expect(made.init.headers, made.path).toEqual({ Authorization: 'Bearer s3cret' })
  }
})

test('a cookie session sends no token: the browser attaches the cookie itself', async () => {
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: cookie, newKey: counted() })
  await api.me()
  await api.runCommand(ID, 3, { type: 'PROPOSE_END' })
  for (const made of calls) expect(made.init.headers).toBeUndefined()
})

test('signing in asks for the session the way this client holds it', async () => {
  for (const session of [cookie, holding(null)]) {
    const { client, calls } = fakeClient([{ status: 200, data: { account: {}, token: 't' } }])
    const api = createExchangeApi({ client, session })
    await api.signIn('ana@example.test', '123456', 'es')
    expect(calls[0].path).toBe('/v1/auth/sessions')
    expect(calls[0].init.body).toEqual({
      identifier: 'ana@example.test',
      code: '123456',
      delivery: session.delivery,
      language: 'es',
      terms_version: TERMS_VERSION,
    })
  }
})

test('with no token there is nobody to ask about, and nothing is sent', async () => {
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: holding(null) })
  expect(await api.me()).toBeNull()
  expect(calls).toEqual([])
})

test('nobody signed in is an answer, not a failure', async () => {
  const { client } = fakeClient([{ status: 401, error: { code: 'UNAUTHENTICATED' } }])
  let signedOut = 0
  const api = createExchangeApi({ client, session: cookie })
  api.onSignedOut(() => (signedOut += 1))
  expect(await api.me()).toBeNull()
  expect(signedOut).toBe(0)
})

test('a session that has ended is reported to whoever holds it', async () => {
  const { client } = fakeClient([{ status: 401, error: { code: 'UNAUTHENTICATED' } }])
  let signedOut = 0
  const api = createExchangeApi({ client, session: holding('old') })
  api.onSignedOut(() => (signedOut += 1))
  await expect(api.listExchanges()).rejects.toMatchObject({ code: 'UNAUTHENTICATED' })
  expect(signedOut).toBe(1)
})

test('a refusal carries the service’s code; anything else is the service being unavailable', async () => {
  const { client } = fakeClient([
    { status: 409, error: { code: 'VERSION_CONFLICT' } },
    { status: 429, error: { code: 'TOO_MANY_REQUESTS' } },
    { status: 502, error: '<html>Bad gateway</html>' },
    'no reply',
  ])
  const api = createExchangeApi({ client, session: cookie, newKey: counted() })
  const command = { type: 'PROPOSE_END' } as const
  await expect(api.runCommand(ID, 1, command)).rejects.toMatchObject({
    code: 'VERSION_CONFLICT',
    unanswered: false,
  })
  await expect(api.runCommand(ID, 1, command)).rejects.toMatchObject({ code: 'TOO_MANY_REQUESTS' })
  await expect(api.runCommand(ID, 1, command)).rejects.toMatchObject({
    code: 'SERVICE_UNAVAILABLE',
    unanswered: true,
  })
  await expect(api.runCommand(ID, 1, command)).rejects.toBeInstanceOf(ApiFailure)
})

test('every change carries the version it was based on and a key of its own', async () => {
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: cookie, newKey: counted() })
  await api.runCommand(ID, 4, { type: 'PROPOSE_END' })
  await api.runCommand(ID, 5, { type: 'CANCEL_END' })
  // The same thing done again later is a new attempt, not a retry.
  await api.runCommand(ID, 4, { type: 'PROPOSE_END' })

  expect(calls.map((made) => made.init.body)).toEqual([
    { expected_version: 4, command: { type: 'PROPOSE_END' } },
    { expected_version: 5, command: { type: 'CANCEL_END' } },
    { expected_version: 4, command: { type: 'PROPOSE_END' } },
  ])
  expect(calls.map((made) => made.init.params?.header?.['Idempotency-Key'])).toEqual([
    'key-1',
    'key-2',
    'key-3',
  ])
})

test('a change that went unanswered is retried with the same key', async () => {
  const { client, calls } = fakeClient(['no reply', { status: 200, data: {} }])
  const api = createExchangeApi({ client, session: cookie, newKey: counted() })
  const command = { type: 'CONFIRM_COUNTERPARTY' } as const
  await expect(api.runCommand(ID, 2, command)).rejects.toMatchObject({ unanswered: true })
  await api.runCommand(ID, 2, command)
  await api.runCommand(ID, 2, command)

  const sent = calls.map((made) => made.init.params?.header?.['Idempotency-Key'])
  expect(sent[1]).toBe(sent[0])
  expect(sent[2]).not.toBe(sent[0])
})

test('sending a revision is keyed the same way', async () => {
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: cookie, newKey: counted() })
  await api.sendRevision(ID, {
    expected_version: 1,
    terms: { party_a_name: 'Ana', party_b_name: 'Ben', terms: '', contributions: [] },
    consent: { language: 'en', version: 'draft-1' },
  })
  expect(calls[0].path).toBe('/v1/exchanges/{id}/revisions')
  expect(calls[0].init.params).toEqual({
    path: { id: ID },
    header: { 'Idempotency-Key': 'key-1' },
  })
})

test('an invitation token travels in the body, never in the address', async () => {
  const token = 'a3'.repeat(32)
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: cookie })
  await api.previewInvitation(token)
  await api.claimInvitation(token)
  await api.invitationAlreadyYours(token)
  for (const made of calls) {
    expect(made.path).not.toContain(token)
    expect(JSON.stringify(made.init.params ?? {})).not.toContain(token)
  }
  expect(calls.map((made) => made.init.body)).toEqual([
    { token },
    { token },
    { token, only_if_yours: true },
  ])
})

test('a device is registered for push and removed with the session’s token', async () => {
  const { client, calls } = fakeClient([
    { status: 200, data: { id: 'd1' } },
    { status: 204 },
  ])
  const api = createExchangeApi({ client, session: holding('s3cret') })
  const device = {
    token: 'ExponentPushToken[abc]',
    platform: 'ios' as const,
    app_version: '0.1.0',
    language: 'es',
  }
  expect(await api.registerDevice(device)).toEqual({ id: 'd1' })
  await api.removeDevice('d1')
  expect(calls.map((made) => `${made.method} ${made.path}`)).toEqual([
    'PUT /v1/me/devices',
    'DELETE /v1/me/devices/{id}',
  ])
  expect(calls[0].init.body).toEqual(device)
  expect(calls[1].init.params?.path).toEqual({ id: 'd1' })
  for (const made of calls) {
    expect(made.init.headers).toEqual({ Authorization: 'Bearer s3cret' })
  }
})

test('reporting and blocking go out like every other call', async () => {
  const token = 'a3'.repeat(32)
  const { client, calls } = fakeClient()
  const api = createExchangeApi({ client, session: holding('s3cret') })
  await api.reportExchange(ID, 'SCAM', 'Asked for payment outside the exchange.')
  await api.reportInvitation(token, 'UNWANTED', null)
  await api.block(ID)
  await api.blockStatus(ID)
  await api.unblock(ID)
  await api.blockedPeople()
  await api.history(ID)
  await api.recordPart(ID, null)

  expect(calls.map((made) => `${made.method} ${made.path}`)).toEqual([
    'POST /v1/exchanges/{id}/reports',
    'POST /v1/invitations/report',
    'PUT /v1/exchanges/{id}/block',
    'GET /v1/exchanges/{id}/block',
    'DELETE /v1/exchanges/{id}/block',
    'GET /v1/blocks',
    'GET /v1/exchanges/{id}/history',
    'GET /v1/exchanges/{id}/record',
  ])
  for (const made of calls) {
    expect(made.init.headers, made.path).toEqual({ Authorization: 'Bearer s3cret' })
    // The invitation token is in a body and nowhere else.
    expect(JSON.stringify(made.init.params ?? {})).not.toContain(token)
  }
  expect(calls[0].init.body).toEqual({
    reason: 'SCAM',
    details: 'Asked for payment outside the exchange.',
  })
  expect(calls[1].init.body).toEqual({ token, reason: 'UNWANTED', details: null })
})
