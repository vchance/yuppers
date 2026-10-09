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

import type { ExchangeSummary, ExchangeView } from '@yuppers/api-client'

import { identifierToSend, phoneProblem, readPhone } from './phone'
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
 * How who an invitation is for was named: by a phone number, by an email
 * address, or not at all (a link for anyone). The step that sends the link
 * leads with the way that reaches them: a text message to the number, an
 * email to the address, or the device's share sheet.
 */
export type InviteeKind = 'phone' | 'email' | 'anyone'

export function inviteeKind(boundTo: string | null): InviteeKind {
  const bound = boundTo?.trim() ?? ''
  if (!bound) return 'anyone'
  if (bound.includes('@')) return validEmail(bound) ? 'email' : 'anyone'
  return phoneNumber(bound) ? 'phone' : 'anyone'
}

/**
 * How long after the sender opened a way to send the link the exchange's
 * page asks them to send it again, while nobody has joined: long enough for
 * the other person to have seen a message, and short enough that a link
 * that never left the sender's phone is not waited on for a week.
 */
export const SHARE_REMINDER_AFTER_MS = 2 * 24 * 60 * 60 * 1000

/**
 * Why the initiator is reminded to send the link: `unsent` because no way to
 * send it was ever opened, `waiting` because that was long ago and nobody
 * has joined.
 */
export type SendReminder = 'unsent' | 'waiting'

/**
 * Whether the initiator is reminded to send the link (DESIGN.md §8). Only
 * they hold one, and only while nobody is in the invited party's place and a
 * link is out: a used or expired link needs replacing, which is said on its
 * own. Yuppers never sends the link, so until the sender has opened a way to
 * send it the other person has nothing; once they have, the reminder comes
 * back only after `SHARE_REMINDER_AFTER_MS` without anyone joining.
 */
export function sendReminder(
  exchange: Pick<
    ExchangeView,
    'you' | 'state' | 'counterparty' | 'invitation_open' | 'invitation_shared_at'
  >,
  now: Date = new Date(),
): SendReminder | null {
  if (exchange.you !== 'A') return null
  if (exchange.state !== 'NEGOTIATING' || exchange.counterparty !== 'UNCLAIMED') return null
  if (exchange.invitation_open === false) return null
  const shared = exchange.invitation_shared_at
  if (!shared) return 'unsent'
  const at = Date.parse(shared)
  if (Number.isNaN(at)) return 'unsent'
  return now.getTime() - at >= SHARE_REMINDER_AFTER_MS ? 'waiting' : null
}

/**
 * What the list says beside an exchange whose link the initiator holds and
 * nobody has joined through: `notSent` while they have not opened a way to
 * send it, `waiting` once they have.
 */
export type InvitationChip = 'notSent' | 'waiting'

export function invitationChip(
  summary: Pick<ExchangeSummary, 'you' | 'state' | 'counterparty' | 'invitation_shared_at'>,
): InvitationChip | null {
  if (summary.you !== 'A') return null
  if (summary.state !== 'NEGOTIATING' || summary.counterparty !== 'UNCLAIMED') return null
  return summary.invitation_shared_at ? 'waiting' : 'notSent'
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
 * The phone number in `input` as the service would keep it, in E.164, or
 * `null` when it is not one: a US number written any usual way, `(202)
 * 555-0142` or `202-555-0142`, with or without +1, or a number given with
 * another country code (`phone.ts`).
 */
function phoneNumber(input: string): string | null {
  const reading = readPhone(input)
  return reading.kind === 'invalid' ? null : reading.phone
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
 * Who an invitation is for, as the service must be told it outright: the
 * person named (a phone number in E.164, however it was typed), or
 * `for_anyone` for a link anyone who has it can claim. The service refuses
 * a request that says neither, so a link for anyone is never what leaving
 * something out gets.
 */
export function invitationOptions(
  boundTo: string | null,
): { bound_to: string } | { for_anyone: true } {
  const named = boundTo?.trim()
  return named ? { bound_to: identifierToSend(named) } : { for_anyone: true }
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
  const problem = phoneProblem(given, channels?.countryCodes ?? [])
  if (emailOnly) return problem === 'invalid' ? 'invalidEmail' : 'emailOnly'
  return problem
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
