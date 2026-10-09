import type { ApiClient } from '@yuppers/api-client'
import { expect, test } from 'vitest'

import { ApiFailure, combineOffer, createExchangeApi, type CombineOffer } from './api'
import { combineAccountLines, combineEffects, combineHeading, shownIdentifier } from './identifiers'
import { wordingFor } from './language'

const en = wordingFor('en')

function offer(over: Partial<CombineOffer> = {}): CombineOffer {
  return {
    token: 't'.repeat(64),
    expires_at: '2026-10-08T12:10:00Z',
    proved: 'PHONE',
    other: {
      display_name: 'Ben',
      email: 'b•••@example.com',
      phone: '(•••) •••-8780',
      yups: { drafts: 1, negotiating: 0, in_force: 2, closed: 3 },
      payment_options: true,
      text_updates: true,
      devices: true,
    },
    email: 'THEIRS_DROPPED',
    phone: 'ADDED',
    payment_options_move: true,
    text_updates_end: false,
    ...over,
  }
}

test('the offer says what the other account is, masked, and its yups', () => {
  expect(combineHeading(en.combine, offer())).toBe(en.combine.headingPhone)
  expect(combineAccountLines(en.combine, offer(), 'en')).toEqual([
    'Name: Ben',
    'Email: b•••@example.com',
    'Phone: (•••) •••-8780',
    '6 yups: 2 in force, 0 waiting for a signature, 1 drafts, 3 closed',
  ])
})

test('the offer says what moves, what is dropped and that the other account ends', () => {
  const lines = combineEffects(en.combine, offer(), 'en')
  expect(lines).toEqual([
    en.combine.movesYups,
    'Its email address, b•••@example.com, is removed: you keep yours.',
    'Its phone number, (•••) •••-8780, is added to this account.',
    en.combine.paymentMove,
    en.combine.textUpdatesMove,
    en.combine.devices,
    en.combine.ends,
  ])
  const kept = combineEffects(
    en.combine,
    offer({
      email: 'REPLACED',
      phone: 'KEPT',
      payment_options_move: false,
      text_updates_end: true,
      other: { ...offer().other, phone: null, devices: false },
    }),
    'en',
  )
  expect(kept).toContain('Its email address, b•••@example.com, replaces yours.')
  expect(kept).toContain(en.combine.paymentDropped)
  expect(kept).toContain(en.combine.textUpdatesEnd)
  expect(kept).not.toContain(en.combine.devices)
})

test('a phone number is shown formatted and an address as it is', () => {
  expect(shownIdentifier('+18565488780')).toBe('(856) 548-8780')
  expect(shownIdentifier('ana@example.com')).toBe('ana@example.com')
})

test('a refusal that offers to combine carries the offer, and the new calls send what the service needs', async () => {
  const calls: { method: string; path: string; init: Record<string, unknown> }[] = []
  const answers = [
    { status: 409, error: { code: 'IDENTIFIER_ON_OTHER_ACCOUNT', combine: offer() } },
    { status: 200, data: {} },
    { status: 200, data: {} },
    { status: 204 },
    { status: 200, data: {} },
  ]
  const call =
    (method: string) =>
    async (path: string, init: Record<string, unknown> = {}) => {
      calls.push({ method, path, init })
      const answer = answers.shift() ?? { status: 200, data: {} }
      const ok = answer.status >= 200 && answer.status < 300
      return {
        data: 'data' in answer ? answer.data : undefined,
        error: 'error' in answer ? answer.error : undefined,
        response: { ok, status: answer.status },
      }
    }
  const client = {
    GET: call('GET'),
    POST: call('POST'),
    PUT: call('PUT'),
    PATCH: call('PATCH'),
    DELETE: call('DELETE'),
  } as unknown as ApiClient
  let next = 0
  const api = createExchangeApi({
    client,
    session: { delivery: 'COOKIE' },
    newKey: () => `key-${(next += 1)}`,
  })

  const refused = await api.addIdentifier('+18565488780', '123456').catch((error) => error)
  expect(refused).toBeInstanceOf(ApiFailure)
  expect(refused.code).toBe('IDENTIFIER_ON_OTHER_ACCOUNT')
  expect(combineOffer(refused)?.token).toBe('t'.repeat(64))
  expect(combineOffer(new ApiFailure('INVALID_CODE'))).toBeNull()

  await api.combineAccounts('t'.repeat(64))
  await api.removeIdentifier('phone', '654321')
  await api.requestInvitationAddressCode('a'.repeat(64), { version: 'v', language: 'en' })
  await api.addInvitationAddress('a'.repeat(64), '111111', true)
  expect(calls.map((each) => `${each.method} ${each.path}`)).toEqual([
    'POST /v1/me/identifiers',
    'POST /v1/me/combine',
    'DELETE /v1/me/identifiers/{kind}',
    'POST /v1/invitations/address/codes',
    'POST /v1/invitations/address',
  ])
  // Both changes carry an idempotency key, and the removal names its kind.
  expect(calls[1].init.params).toEqual({ header: { 'Idempotency-Key': 'key-1' } })
  expect(calls[2].init.params).toEqual({
    path: { kind: 'phone' },
    header: { 'Idempotency-Key': 'key-2' },
  })
  expect(calls[4].init.body).toEqual({ token: 'a'.repeat(64), code: '111111', replace: true })
})
