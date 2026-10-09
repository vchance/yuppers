import type { Account, ErrorCode } from '@yuppers/api-client'
import { useCallback, useRef, useState } from 'react'

import {
  combineOffer,
  failureCode,
  type CombineOffer,
  type ExchangeApi,
  type InAppNotice,
} from './api'
import { formatMessage } from './message'
import { formatPhone, usPhone } from './phone'
import { smsCodeConsent, useSmsCodeConsentBox, type SmsCodeConsentBox } from './sms-code-consent'
import type { Wording } from './wording/types'

/*
 * The account's email address and phone number (README, "Combining
 * accounts"), as both apps show them: each with Add, Change and Remove.
 *
 *   - Adding takes the code sent to the new one (`addIdentifier`).
 *   - Changing takes more than a session: first a code to one of the
 *     account's own, the one being replaced or, if the person no longer has
 *     it, the other (`proveIdentifier`), which gives a proof; then the new
 *     one's code, with the proof. An email address replaced is told by email.
 *   - Removing keeps the other, which must be there: Remove is not offered
 *     for the only one, and the screen says why. It is proved with a code
 *     sent to the one that stays, so the person knows they can still sign in.
 *   - If the code shows the address is on another of the person's accounts,
 *     the service offers to combine the two (`IDENTIFIER_ON_OTHER_ACCOUNT`),
 *     with what that account holds and what combining would do; combining
 *     takes one more, deliberate, confirmation.
 *
 * Until a code is right, an address with an account is answered like any
 * other: nothing here can find out whether someone has one.
 */

export type IdentifierSlot = 'email' | 'phone'

export type IdentifiersApi = Pick<
  ExchangeApi,
  'requestCode' | 'addIdentifier' | 'removeIdentifier' | 'combineAccounts' | 'proveIdentifier'
>

/** What is being done to one of the two, and how far it has got. */
export type IdentifierEdit =
  /** Changing: where the code proving one of the account's own goes. */
  | { slot: IdentifierSlot; action: 'change'; step: 'prove'; to: string }
  /** That code was sent to `to`. */
  | { slot: IdentifierSlot; action: 'change'; step: 'proveCode'; to: string }
  /** The new address or number to type. */
  | { slot: IdentifierSlot; action: 'add' | 'change'; step: 'enter' }
  /** A code was sent to `identifier`, as the service takes it. */
  | {
      slot: IdentifierSlot
      action: 'add' | 'change'
      step: 'code'
      identifier: string
    }
  /** What removing does, and where the code will go. */
  | { slot: IdentifierSlot; action: 'remove'; step: 'explain'; staying: string }
  /** The code was sent to `staying`, the one that stays. */
  | { slot: IdentifierSlot; action: 'remove'; step: 'code'; staying: string }

/** An identifier as the screens show it: a phone number formatted. */
export function shownIdentifier(value: string): string {
  return value.includes('@') ? value : formatPhone(value)
}

export interface IdentifiersControl {
  edit: IdentifierEdit | null
  /** What is typed in the address or number field. */
  input: string
  setInput(input: string): void
  code: string
  setCode(code: string): void
  /** The number typed is not a US number. */
  invalidPhone: boolean
  /** The box beside a number a code would be texted to. */
  codeConsent: SmsCodeConsentBox
  busy: boolean
  failure: ErrorCode | null
  /** The offer to combine, once a code showed the address is another account's. */
  offer: CombineOffer | null
  /** What was just done, for the screen to say. */
  done: string | null
  /**
   * While proving one of the account's own: the account's other one, to
   * send the code there instead, if it has one.
   */
  proveAlternative: string | null
  /** Sends the proving code to `proveAlternative` instead. */
  proveElsewhere(): void
  /** Whether Remove can be offered for this one: the account has the other. */
  removable(slot: IdentifierSlot): boolean
  start(slot: IdentifierSlot, action: 'add' | 'change' | 'remove'): void
  cancel(): void
  sendCode(): Promise<void>
  confirm(): Promise<void>
  combine(): Promise<void>
  dismissOffer(): void
}

/**
 * The control for the account's email address and phone number.
 * `onAccount` is told whenever the account changes, so the client's view of
 * it follows; `language` is the one the box beside a number is shown in.
 */
export function useIdentifiers(
  api: IdentifiersApi,
  account: Pick<Account, 'email' | 'phone'>,
  onAccount: (account: Account) => void,
  wording: Wording,
  language: string,
): IdentifiersControl {
  const [edit, setEdit] = useState<IdentifierEdit | null>(null)
  const [input, setInputState] = useState('')
  const [code, setCode] = useState('')
  const [invalidPhone, setInvalidPhone] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [offer, setOffer] = useState<CombineOffer | null>(null)
  const [done, setDone] = useState<string | null>(null)
  // A proof of one of the account's own, once its code was entered: what
  // replacing one, and a combination that does, takes.
  const [proof, setProof] = useState<string | null>(null)
  const working = useRef(false)
  const w = wording.identifiers

  // A code by text needs the box beside the number ticked: the number
  // typed for a phone being added, or the account's own when its email is
  // being removed.
  const textedTo =
    edit?.slot === 'phone' && edit.action !== 'remove' && edit.step === 'enter'
      ? (usPhone(input) ?? (input.trim() === '' ? null : input.trim()))
      : edit?.action === 'remove' && edit.step === 'explain' && edit.slot === 'email'
        ? edit.staying
        : edit?.step === 'prove' && !edit.to.includes('@')
          ? edit.to
          : null
  const codeConsent = useSmsCodeConsentBox(textedTo)

  const run = useCallback(async (work: () => Promise<void>) => {
    if (working.current) return
    working.current = true
    setBusy(true)
    setFailure(null)
    try {
      await work()
    } catch (error) {
      const offered = combineOffer(error)
      if (offered) {
        setOffer(offered)
        setEdit(null)
        setCode('')
      } else {
        setFailure(failureCode(error))
      }
    } finally {
      working.current = false
      setBusy(false)
    }
  }, [])

  const other = (slot: IdentifierSlot) => (slot === 'email' ? account.phone : account.email)
  const own = (slot: IdentifierSlot) => (slot === 'email' ? account.email : account.phone)
  const proveAlternative =
    edit?.step === 'prove'
      ? ([account.email, account.phone].find((value) => value && value !== edit.to) ?? null)
      : null

  return {
    edit,
    input,
    setInput: (value: string) => {
      setInputState(value)
      setInvalidPhone(false)
    },
    code,
    setCode,
    invalidPhone,
    codeConsent,
    busy,
    failure,
    offer,
    done,
    proveAlternative,
    proveElsewhere() {
      if (edit?.step !== 'prove' || !proveAlternative) return
      setFailure(null)
      setEdit({ ...edit, to: proveAlternative })
    },
    removable: (slot) =>
      Boolean(other(slot)) && Boolean(slot === 'email' ? account.email : account.phone),
    start(slot, action) {
      setFailure(null)
      setDone(null)
      setOffer(null)
      setInputState('')
      setCode('')
      setInvalidPhone(false)
      setProof(null)
      const current = own(slot)
      if (action === 'remove') {
        const staying = other(slot)
        if (!staying) return
        setEdit({ slot, action, step: 'explain', staying })
      } else if (action === 'change' && current) {
        setEdit({ slot, action, step: 'prove', to: current })
      } else {
        setEdit({ slot, action, step: 'enter' })
      }
    },
    cancel() {
      setEdit(null)
      setFailure(null)
      setCode('')
    },
    async sendCode() {
      if (!edit) return
      if (edit.step === 'prove' || edit.step === 'proveCode') {
        if (edit.step === 'prove' && codeConsent.missing) return
        const to = edit.to
        await run(async () => {
          await api.requestCode(to, to.includes('@') ? undefined : smsCodeConsent(language))
          setEdit({ slot: edit.slot, action: 'change', step: 'proveCode', to })
          setCode('')
        })
        return
      }
      if (edit.action === 'remove') {
        if (codeConsent.missing) return
        const staying = edit.staying
        await run(async () => {
          await api.requestCode(
            staying,
            staying.includes('@') ? undefined : smsCodeConsent(language),
          )
          setEdit({ ...edit, step: 'code' })
        })
        return
      }
      let identifier: string
      if (edit.slot === 'phone') {
        const phone = usPhone(input)
        setInvalidPhone(phone === null)
        if (phone === null || codeConsent.missing) return
        identifier = phone
      } else {
        identifier = input.trim()
      }
      await run(async () => {
        await api.requestCode(
          identifier,
          edit.slot === 'phone' ? smsCodeConsent(language) : undefined,
        )
        setEdit({
          slot: edit.slot,
          action: edit.action,
          step: 'code',
          identifier,
        })
        setCode('')
      })
    },
    async confirm() {
      if (edit?.step === 'proveCode') {
        const channel = edit.to.includes('@') ? 'EMAIL' : 'PHONE'
        await run(async () => {
          const proved = await api.proveIdentifier(channel, code.trim())
          setProof(proved.proof)
          setEdit({ slot: edit.slot, action: 'change', step: 'enter' })
          setCode('')
        })
        return
      }
      if (!edit || edit.step !== 'code') return
      if (edit.action === 'remove') {
        await run(async () => {
          const changed = await api.removeIdentifier(edit.slot, code.trim())
          onAccount(changed)
          setEdit(null)
          setCode('')
          setDone(w.removed)
        })
        return
      }
      const identifier = edit.identifier
      await run(async () => {
        const changed = await api.addIdentifier(identifier, code.trim(), proof ?? undefined)
        onAccount(changed)
        setEdit(null)
        setCode('')
        setProof(null)
        setDone(formatMessage(w.added, { identifier: shownIdentifier(identifier) }, language))
      })
    },
    async combine() {
      if (!offer) return
      const token = offer.token
      await run(async () => {
        // The proof given with the address found it on the other account,
        // and was not used up: a combination that replaces one of this
        // account's own takes it.
        const changed = await api.combineAccounts(
          token,
          offer.proof_required ? (proof ?? undefined) : undefined,
        )
        onAccount(changed)
        setOffer(null)
        setProof(null)
        setDone(wording.combine.done)
      })
    },
    dismissOffer() {
      setOffer(null)
      setFailure(null)
    },
  }
}

/**
 * What a notice in the app says (`notice` in `GET /v1/me`), with `date`, the
 * time it happened as the screens write it.
 */
export function noticeText(wording: Wording, kind: InAppNotice, date: string, language: string): string {
  const text =
    kind === 'PHONE_CHANGED'
      ? wording.identifiers.noticePhoneChanged
      : kind === 'PHONE_REMOVED'
        ? wording.identifiers.noticePhoneRemoved
        : wording.combine.noticeBanner
  return formatMessage(text, { date }, language)
}

/** The heading of the offer: which kind of address was proved. */
export function combineHeading(w: Wording['combine'], offer: CombineOffer): string {
  return offer.proved === 'PHONE' ? w.headingPhone : w.headingEmail
}

/** What the other account is, line by line: its name, address, number and yups. */
export function combineAccountLines(
  w: Wording['combine'],
  offer: CombineOffer,
  language: string,
): string[] {
  const other = offer.other
  const lines: string[] = []
  if (other.display_name)
    lines.push(formatMessage(w.otherName, { name: other.display_name }, language))
  if (other.email) lines.push(formatMessage(w.otherEmail, { identifier: other.email }, language))
  if (other.phone) lines.push(formatMessage(w.otherPhone, { identifier: other.phone }, language))
  const { drafts, negotiating, in_force: inForce, closed } = other.yups
  const total = drafts + negotiating + inForce + closed
  const yups = formatMessage(w.yups, { count: total }, language)
  lines.push(
    total === 0
      ? yups
      : `${yups}: ${formatMessage(w.yupsDetail, { inForce, negotiating, drafts, closed }, language)}`,
  )
  return lines
}

/** What combining does, line by line, from the offer. */
export function combineEffects(
  w: Wording['combine'],
  offer: CombineOffer,
  language: string,
): string[] {
  const lines = [w.movesYups]
  const other = offer.other
  const identifier = (slot: IdentifierSlot) => (slot === 'email' ? other.email : other.phone) ?? ''
  for (const slot of ['email', 'phone'] as const) {
    const outcome = offer[slot]
    const words =
      slot === 'email'
        ? {
            ADDED: w.emailAdded,
            REPLACED: w.emailReplaced,
            THEIRS_DROPPED: w.emailDropped,
          }
        : {
            ADDED: w.phoneAdded,
            REPLACED: w.phoneReplaced,
            THEIRS_DROPPED: w.phoneDropped,
          }
    if (outcome === 'ADDED' || outcome === 'REPLACED' || outcome === 'THEIRS_DROPPED') {
      lines.push(formatMessage(words[outcome], { identifier: identifier(slot) }, language))
    }
  }
  if (other.payment_options) {
    lines.push(offer.payment_options_move ? w.paymentMove : w.paymentDropped)
  }
  if (other.text_updates) {
    lines.push(offer.text_updates_end ? w.textUpdatesEnd : w.textUpdatesMove)
  }
  if (other.devices) lines.push(w.devices)
  lines.push(w.ends)
  return lines
}
