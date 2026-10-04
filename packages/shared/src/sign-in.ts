import type { components } from '@yuppers/api-client'
import { useEffect, useState } from 'react'

import type { ExchangeApi } from './api'
import type { MessageValues } from './message'
import type { Wording } from './wording/types'

/*
 * What someone can sign in with. The service says which kinds of identifier
 * it can send a code to right now (`GET /v1/meta`, `sign_in_channels`): a
 * deployment without text messages takes email addresses only. Both apps ask
 * for what can be used, and stop a phone number typed anyway before it is
 * sent, with words that say what to do instead of "service unavailable".
 */

export type SignInChannel = components['schemas']['SignInChannel']

/** What the service said it can send codes to, and how. */
export interface SignInChannels {
  phone: boolean
  /** The country calling codes phone numbers may have, such as `+1`. */
  countryCodes: string[]
  /**
   * The email address codes sent by email come from (`code_sender`), or
   * `null` where the service does not say, as in development.
   */
  codeSender: string | null
}

export type SignInApi = Pick<ExchangeApi, 'meta'>

/**
 * What the service can send codes to, or `null` when it cannot say: it did
 * not answer, or is too old to know.
 */
export function signInChannels(api: SignInApi): Promise<SignInChannels | null> {
  return api.meta().then(
    (meta) => {
      // Partial, for a service from before the field existed.
      const said = meta as Partial<typeof meta>
      if (!Array.isArray(said.sign_in_channels)) return null
      return {
        phone: said.sign_in_channels.includes('phone'),
        countryCodes: said.sms_country_codes ?? [],
        codeSender: said.code_sender ?? null,
      }
    },
    () => null,
  )
}

/** [`signInChannels`], asked once each time a form that needs it is shown. */
export function useSignInChannels(api: SignInApi): SignInChannels | null {
  const [channels, setChannels] = useState<SignInChannels | null>(null)
  useEffect(() => {
    let cancelled = false
    void signInChannels(api).then((found) => {
      if (!cancelled) setChannels(found)
    })
    return () => {
      cancelled = true
    }
  }, [api])
  return channels
}

/** Whether a phone number may be offered. Not until the service has said so. */
export function phoneOffered(channels: SignInChannels | null): boolean {
  return channels?.phone === true
}

/**
 * Why `input` is not sent, if it is not: something other than an email
 * address where the service has said it takes email addresses only. Only
 * once it has said: while it cannot, the service decides as before.
 */
export function identifierRefused(
  input: string,
  channels: SignInChannels | null,
): 'emailOnly' | null {
  if (channels === null || channels.phone) return null
  return input.includes('@') ? null : 'emailOnly'
}

/** The words of the sign-in form for what the service can send codes to. */
export interface SignInText {
  intro: string
  label: string
  hint: string | undefined
  changeIdentifier: string
}

export function signInText(
  w: Wording['signIn'],
  channels: SignInChannels | null,
  fmt: (message: string, values: MessageValues) => string,
): SignInText {
  if (!phoneOffered(channels)) {
    return {
      intro: w.introEmail,
      label: w.emailLabel,
      hint: undefined,
      changeIdentifier: w.changeEmail,
    }
  }
  const codes = channels?.countryCodes ?? []
  return {
    intro: w.intro,
    label: w.identifierLabel,
    hint: codes.length ? fmt(w.identifierHintCountries, { codes: codes.join(', ') }) : w.identifierHint,
    changeIdentifier: w.changeIdentifier,
  }
}

/*
 * Waiting for a code. A code by email can land in a spam folder, so the
 * form says where to look and whom it comes from. Asking again is offered
 * only once the first has had time to arrive: a request is counted against
 * the hourly limits whether or not the email was merely slow, and asking
 * at once, again and again, would use them up for nothing.
 */

/** How long after a code is sent asking for another is offered. */
export const RESEND_AFTER_MS = 30_000

/**
 * What to look for after a code was sent to `sentTo`: the inbox and the spam
 * folder, and the address it comes from where the service says. `null` for
 * a phone number, whose text message needs no such help.
 */
export function codeWaitText(
  w: Wording['signIn'],
  sentTo: string,
  channels: SignInChannels | null,
  fmt: (message: string, values: MessageValues) => string,
): string | null {
  if (!sentTo.includes('@')) return null
  const sender = channels?.codeSender
  return sender ? fmt(w.checkInbox, { sender }) : w.checkInboxAnySender
}

/**
 * Whether asking for another code is offered: [`RESEND_AFTER_MS`] after
 * `sentAt`, the time the latest code was sent, changes. `null` while none
 * has been.
 */
export function useResendReady(sentAt: number | null): boolean {
  const [ready, setReady] = useState<number | null>(null)
  useEffect(() => {
    if (sentAt === null) return
    const timer = setTimeout(() => setReady(sentAt), RESEND_AFTER_MS)
    return () => clearTimeout(timer)
  }, [sentAt])
  return sentAt !== null && ready === sentAt
}
