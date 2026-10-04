import type { ApiClient } from '@yuppers/api-client'
import { expect, test } from 'vitest'

import { ApiFailure, createExchangeApi } from './api'
import { codeDestinations, deletedNotice, deleteWithCode } from './deletion'

interface Call {
  method: string
  path: string
  init: { headers?: Record<string, string>; body?: unknown }
}

type Answer = { status: number; data?: unknown; error?: unknown }

/** A stand-in for the generated client: records each call and answers from a script. */
function fakeClient(answers: Answer[] = []) {
  const calls: Call[] = []
  const call =
    (method: string) =>
    async (path: string, init: Call['init'] = {}) => {
      calls.push({ method, path, init })
      const answer = answers.shift() ?? { status: 204 }
      const ok = answer.status >= 200 && answer.status < 300
      return { data: answer.data, error: answer.error, response: { ok, status: answer.status } }
    }
  const client = { GET: call('GET'), POST: call('POST') } as unknown as ApiClient
  return { client, calls }
}

test('a code can go to whichever identifiers the account has, email first', () => {
  expect(codeDestinations({ email: 'ana@example.test', phone: '+12025550142' })).toEqual([
    { channel: 'EMAIL', identifier: 'ana@example.test' },
    { channel: 'PHONE', identifier: '+12025550142' },
  ])
  expect(codeDestinations({ email: null, phone: '+12025550142' })).toEqual([
    { channel: 'PHONE', identifier: '+12025550142' },
  ])
  expect(codeDestinations({ email: 'ana@example.test' })).toEqual([
    { channel: 'EMAIL', identifier: 'ana@example.test' },
  ])
  expect(codeDestinations({})).toEqual([])
})

test('a phone number the service cannot text is not offered, unless it is all there is', () => {
  const both = { email: 'ana@example.test', phone: '+12025550142' }
  const emailOnly = { phone: false, countryCodes: [], codeSender: null }
  expect(codeDestinations(both, emailOnly)).toEqual([
    { channel: 'EMAIL', identifier: 'ana@example.test' },
  ])
  expect(codeDestinations(both, { phone: true, countryCodes: ['+1'], codeSender: null })).toHaveLength(2)
  // Not yet known: both, as before.
  expect(codeDestinations(both, null)).toHaveLength(2)
  expect(codeDestinations({ email: null, phone: '+12025550142' }, emailOnly)).toEqual([
    { channel: 'PHONE', identifier: '+12025550142' },
  ])
})

test('the calls say which identifier, never the address, and carry the session', async () => {
  const preview = { drafts: 1, open_proposals: 0, agreements_in_force: 2 }
  const { client, calls } = fakeClient([{ status: 200, data: preview }])
  const api = createExchangeApi({
    client,
    session: { delivery: 'TOKEN', token: () => 's3cret' },
  })

  expect(await api.deletionPreview()).toEqual(preview)
  await api.requestDeletionCode('PHONE')
  await api.deleteAccount('PHONE', '123456')

  const authorized = { Authorization: 'Bearer s3cret' }
  expect(calls).toEqual([
    { method: 'GET', path: '/v1/me/deletion', init: { headers: authorized } },
    {
      method: 'POST',
      path: '/v1/me/deletion/codes',
      init: { headers: authorized, body: { channel: 'PHONE' } },
    },
    {
      method: 'POST',
      path: '/v1/me/deletion',
      init: { headers: authorized, body: { channel: 'PHONE', code: '123456' } },
    },
  ])
})

test('a wrong code is not the end of the session; a dead session is', async () => {
  for (const [code, ends] of [
    ['INVALID_CODE', false],
    ['UNAUTHENTICATED', true],
  ] as const) {
    const { client } = fakeClient([{ status: 401, error: { code } }])
    const api = createExchangeApi({ client, session: { delivery: 'COOKIE' } })
    let signedOut = false
    api.onSignedOut(() => {
      signedOut = true
    })
    await expect(api.deleteAccount('EMAIL', '000000')).rejects.toMatchObject({ code })
    expect(signedOut).toBe(ends)
  }
})

test('deleting says whether it happened, and sends the code as typed apart from spaces', async () => {
  const sent: unknown[] = []
  const api = {
    deleteAccount: async (...args: unknown[]) => {
      sent.push(args)
    },
  }
  expect(await deleteWithCode(api, 'EMAIL', ' 123456 ')).toEqual({ deleted: true })
  expect(sent).toEqual([['EMAIL', '123456']])
})

test('a code that was not accepted goes back to the step that asks for it', async () => {
  const refusing = (code: ConstructorParameters<typeof ApiFailure>[0]) => ({
    deleteAccount: () => Promise.reject(new ApiFailure(code)),
  })
  expect(await deleteWithCode(refusing('INVALID_CODE'), 'EMAIL', '000000')).toEqual({
    deleted: false,
    code: 'INVALID_CODE',
    retype: true,
  })
  // Anything else leaves the person where they were, to try again.
  for (const code of [
    'SERVICE_UNAVAILABLE',
    'TOO_MANY_REQUESTS',
    'TOO_MANY_GUESSES',
    'UNAUTHENTICATED',
  ] as const) {
    expect(await deleteWithCode(refusing(code), 'EMAIL', '123456')).toEqual({
      deleted: false,
      code,
      retype: false,
    })
  }
  const broken = { deleteAccount: () => Promise.reject(new TypeError('boom')) }
  expect(await deleteWithCode(broken, 'EMAIL', '123456')).toEqual({
    deleted: false,
    code: 'INTERNAL',
    retype: false,
  })
})

test('that the account was deleted is said until it is dismissed', () => {
  let changes = 0
  const stop = deletedNotice.subscribe(() => {
    changes += 1
  })
  expect(deletedNotice.shown()).toBe(false)

  deletedNotice.announce()
  deletedNotice.announce()
  expect(deletedNotice.shown()).toBe(true)
  expect(changes).toBe(1)

  deletedNotice.dismiss()
  expect(deletedNotice.shown()).toBe(false)
  expect(changes).toBe(2)

  stop()
  deletedNotice.announce()
  expect(changes).toBe(2)
  deletedNotice.dismiss()
})
