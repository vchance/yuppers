import type { Meta } from '@yuppers/api-client'
import { expect, test } from 'vitest'

import { ApiFailure } from './api'
import { wordingFor } from './language'
import { formatMessage, type MessageValues } from './message'
import {
  codeWaitText,
  identifierRefused,
  RESEND_AFTER_MS,
  signInChannels,
  signInText,
  type SignInApi,
} from './sign-in'

const META: Meta = {
  service: 'yuppers-backend',
  version: '0.0.0',
  commit: 'unknown',
  minimum_client_versions: {},
  push_notifications: false,
  wallet_platforms: [],
  sign_in_channels: ['email'],
  sms_country_codes: [],
  sms_updates: false,
}

function answering(meta: Partial<Meta> | Error): SignInApi {
  return {
    meta: async () => {
      if (meta instanceof Error) throw meta
      return { ...META, ...meta }
    },
  }
}

test('the service says what it can send codes to', async () => {
  expect(await signInChannels(answering({}))).toEqual({ phone: false, countryCodes: [], codeSender: null })
  expect(
    await signInChannels(
      answering({ sign_in_channels: ['email', 'phone'], sms_country_codes: ['+1', '+52'] }),
    ),
  ).toEqual({ phone: true, countryCodes: ['+1', '+52'], codeSender: null })
  // A service that cannot say, or one from before it could: not known.
  expect(await signInChannels(answering(new ApiFailure('SERVICE_UNAVAILABLE')))).toBeNull()
  const { sign_in_channels: _channels, sms_country_codes: _codes, ...older } = META
  expect(await signInChannels({ meta: async () => older as Meta })).toBeNull()
})

test('a phone number is stopped only once the service has said it takes email only', () => {
  const emailOnly = { phone: false, countryCodes: [], codeSender: null }
  expect(identifierRefused('+1 555 123 4567', emailOnly)).toBe('emailOnly')
  expect(identifierRefused('5551234567', emailOnly)).toBe('emailOnly')
  expect(identifierRefused('ana@example.test', emailOnly)).toBeNull()
  expect(identifierRefused('+15551234567', { phone: true, countryCodes: ['+1'], codeSender: null })).toBeNull()
  // Not known: the service decides, as before.
  expect(identifierRefused('+15551234567', null)).toBeNull()
})

test('the form says email only until the service offers phone numbers, then names the countries', () => {
  const w = wordingFor('en').signIn
  const fmt = (message: string, values: MessageValues) => formatMessage(message, values, 'en')
  expect(signInText(w, null, fmt)).toEqual({
    intro: w.introEmail,
    label: w.emailLabel,
    hint: undefined,
    changeIdentifier: w.changeEmail,
  })
  expect(signInText(w, { phone: false, countryCodes: [], codeSender: null }, fmt).label).toBe(w.emailLabel)
  expect(signInText(w, { phone: true, countryCodes: ['+1', '+52'], codeSender: null }, fmt)).toEqual({
    intro: w.intro,
    label: w.identifierLabel,
    hint: 'For a phone number, start with the country code. We can only send codes to phone numbers starting +1, +52.',
    changeIdentifier: w.changeIdentifier,
  })
  expect(signInText(w, { phone: true, countryCodes: [], codeSender: null }, fmt).hint).toBe(w.identifierHint)
})

test('the service names the address codes come from, where it has one', async () => {
  expect(await signInChannels(answering({ code_sender: 'no-reply@yuppers.example' }))).toEqual({
    phone: false,
    countryCodes: [],
    codeSender: 'no-reply@yuppers.example',
  })
  expect((await signInChannels(answering({ code_sender: null })))?.codeSender).toBeNull()
})

test('waiting for a code by email says to look in the spam folder too, and for whom', () => {
  const w = wordingFor('en').signIn
  const fmt = (message: string, values: MessageValues) => formatMessage(message, values, 'en')
  const named = { phone: true, countryCodes: ['+1'], codeSender: 'no-reply@yuppers.example' }
  expect(codeWaitText(w, 'ana@example.test', named, fmt)).toBe(
    'Check your inbox, and your spam folder, for an email from no-reply@yuppers.example.',
  )
  // Where the service does not say, or cannot be asked: the folders alone.
  const unnamed = { phone: false, countryCodes: [], codeSender: null }
  expect(codeWaitText(w, 'ana@example.test', unnamed, fmt)).toBe(w.checkInboxAnySender)
  expect(codeWaitText(w, 'ana@example.test', null, fmt)).toBe(w.checkInboxAnySender)
  // A text message needs none of it.
  expect(codeWaitText(w, '+15551234567', named, fmt)).toBeNull()
  // Another code is offered half a minute after the last.
  expect(RESEND_AFTER_MS).toBe(30_000)
})
