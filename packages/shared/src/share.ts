/*
 * Passing an invitation link on (DESIGN.md §8). The service never sends an
 * invitation itself: sending to whatever address someone types would let it
 * be used to spam or harass, and would wear down the sending domain's
 * reputation. So the initiator passes the link on through a channel of their
 * own, and these are the ways the apps offer to start one.
 *
 * The token stays in the link's fragment. The email, text message and
 * WhatsApp addresses built here carry the whole link percent-encoded, `#`
 * included, inside their own query, and are never sent to this service. The
 * `mailto:` and `sms:` ones are opened on the device, by a mail app or the
 * messages app. The WhatsApp one is an `https://wa.me/` address: where the
 * WhatsApp app is installed the device hands it to the app, and where it is
 * not (a computer, a phone without it) the browser loads it from WhatsApp's
 * web server, which then receives the whole link, token included, in that
 * query. Whichever channel is chosen sees the link, as any channel the
 * person picks would.
 */

import { phoneOffered, type SignInChannels } from './sign-in'
import type { MessageValues } from './message'
import type { Wording } from './wording/types'

/**
 * An invitation just issued, held only while its screen stays open: the
 * token is shown once and cannot be fetched again. `boundTo` is who it was
 * made for, as typed, when it names someone.
 */
export interface IssuedInvitation {
  token: string
  boundTo: string | null
}

/** The addresses that start a message carrying the link, each in its own app. */
export interface ShareAddresses {
  email: string
  sms: string
  whatsApp: string
}

/**
 * Where to start each kind of message. `message` is the text with the link
 * in it; `subject` an email's subject. An invitation made for an email
 * address addresses the email to it, and one made for a phone number the
 * text message, so the person does not type it twice.
 */
export function shareAddresses({
  message,
  subject,
  boundTo,
}: {
  message: string
  subject: string
  boundTo: string | null
}): ShareAddresses {
  const body = encodeURIComponent(message)
  const bound = boundTo?.trim() ?? ''
  const email = bound.includes('@') && validEmail(bound) ? bound : ''
  const phone = email ? null : phoneNumber(bound)
  // RFC 6068: the address is percent-encoded but for its `@`, which some
  // mail apps do not decode.
  const to = encodeURIComponent(email).replace('%40', '@')
  return {
    email: `mailto:${to}?subject=${encodeURIComponent(subject)}&body=${body}`,
    sms: `sms:${phone ?? ''}?body=${body}`,
    whatsApp: `https://wa.me/?text=${body}`,
  }
}

/**
 * Whether `input` looks like an email address, by the same rule the service
 * applies (`backend/src/domain/identity.rs`): ASCII, a local part, and a
 * domain with a dot and no empty labels.
 */
function validEmail(input: string): boolean {
  const email = input.trim()
  const at = email.lastIndexOf('@')
  if (at < 1) return false
  const local = email.slice(0, at)
  const domain = email.slice(at + 1)
  return (
    // oxlint-disable-next-line no-control-regex -- ASCII is what is checked.
    /^[\x00-\x7f]*$/.test(email) &&
    email.length <= 254 &&
    !local.includes('@') &&
    domain.includes('.') &&
    domain.split('.').every((label) => label.length > 0) &&
    // oxlint-disable-next-line no-control-regex -- control characters are what is refused.
    !/[\s\x00-\x1f\x7f]/.test(email)
  )
}

/**
 * The phone number in `input` as the service would keep it, `+` and its
 * digits, or `null` when it is not one by the service's rule: a `+`, then
 * 7 to 15 digits not starting with 0, written with spaces, dashes, dots or
 * brackets if at all.
 */
function phoneNumber(input: string): string | null {
  const given = input.trim()
  if (!given.startsWith('+')) return null
  const rest = given.slice(1)
  if (!/^[0-9 ().-]*$/.test(rest)) return null
  const digits = rest.replace(/[^0-9]/g, '')
  return digits.length >= 7 && digits.length <= 15 && !digits.startsWith('0') ? `+${digits}` : null
}

/**
 * Why who an invitation is for would not do, if it would not. `missing` is
 * only for [`invitationForProblem`]: nobody named, and a link for anyone
 * not chosen either.
 */
export type BoundToProblem = 'missing' | 'invalid' | 'invalidEmail' | 'emailOnly' | 'country'

/**
 * Who an invitation is for, as the composer and the panel that replaces a
 * link ask it (DESIGN.md §8). Naming the person is what is expected: a named
 * invitation needs no confirmation, so they can respond in full as soon as
 * they sign in. A link for anyone is a deliberate choice, never what an
 * empty field means. What was typed is kept while a link for anyone is
 * chosen, so changing one's mind back loses nothing.
 */
export interface InvitationChoice {
  anyone: boolean
  /** Their email address or phone number, as typed. */
  to: string
}

/** Where the question starts: naming them, with nothing typed yet. */
export const NAMED_INVITATION: InvitationChoice = { anyone: false, to: '' }

/**
 * What is wrong with an [`InvitationChoice`], if anything: nothing for a
 * link for anyone; otherwise someone has to be named, by an address they can
 * sign in with ([`boundToProblem`]).
 */
export function invitationForProblem(
  choice: InvitationChoice,
  channels: SignInChannels | null,
): BoundToProblem | null {
  if (choice.anyone) return null
  if (!choice.to.trim()) return 'missing'
  return boundToProblem(choice.to, channels)
}

/** Whom the service is asked to bind the invitation to: nobody for a link for anyone. */
export function invitationBoundTo(choice: InvitationChoice): string | null {
  return choice.anyone ? null : choice.to.trim() || null
}

/**
 * Checks who an invitation is for before anything is signed, gently: only
 * what is plainly not an email address or phone number, or one the person
 * could never sign in with here. Empty is fine; it names nobody. Where the
 * service takes email addresses only (`GET /v1/meta`), a phone number is
 * refused, since the person it names could not sign in with it to use the
 * link; and a phone number of a country the service does not text, for the
 * same reason. While the service has not said, only the shape is checked.
 */
export function boundToProblem(
  input: string,
  channels: SignInChannels | null,
): BoundToProblem | null {
  const given = input.trim()
  if (!given) return null
  const emailOnly = channels !== null && !channels.phone
  if (given.includes('@')) {
    if (validEmail(given)) return null
    return emailOnly ? 'invalidEmail' : 'invalid'
  }
  const phone = phoneNumber(given)
  if (emailOnly) return phone ? 'emailOnly' : 'invalidEmail'
  if (!phone) return 'invalid'
  const codes = channels?.countryCodes ?? []
  if (codes.length > 0 && !codes.some((code) => phone.startsWith(code))) return 'country'
  return null
}

/** The words for a [`BoundToProblem`]. */
export function boundToProblemText(
  problem: BoundToProblem,
  w: Wording['invitationLink'],
  channels: SignInChannels | null,
  fmt: (message: string, values: MessageValues) => string,
): string {
  switch (problem) {
    case 'missing':
      return phoneOffered(channels) ? w.forMissing : w.forMissingEmail
    case 'invalid':
      return w.forInvalid
    case 'invalidEmail':
      return w.forInvalidEmail
    case 'emailOnly':
      return w.forEmailOnly
    case 'country':
      return fmt(w.forCountry, { codes: (channels?.countryCodes ?? []).join(', ') })
  }
}

/**
 * The label for who an invitation is for: email or phone where the service
 * texts codes, and email alone until it has said so, as signing in asks.
 */
export function boundToLabel(
  w: Wording['invitationLink'],
  channels: SignInChannels | null,
): string {
  return phoneOffered(channels) ? w.forLabel : w.forLabelEmail
}
