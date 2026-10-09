import type { Account, ErrorCode, ExchangeView } from '@yuppers/api-client'
import { useCallback, useEffect, useRef, useState } from 'react'

import { failureCode, type ExchangeApi, type SmsUpdates } from './api'
import { usPhone } from './phone'
import { smsCodeConsent } from './sms-code-consent'

/*
 * Text updates for an agreement: "Yuppers.app agreement updates" (DESIGN.md
 * §12). On an agreement someone is a party to, a "Text updates" control:
 *
 *   - with no phone number on the account, a US number to add, checked with
 *     a one-time code by text (`requestCode`, then `addIdentifier`, the way
 *     any identifier is added), asked for only once the box beside the
 *     number is ticked (`smsCode.verifyNumber`, `sms-code-consent.ts`); and,
 *     as adding any takes, a code to the account's email address with it,
 *     which proves the person is its holder (`proveIdentifier`);
 *   - with one, a box beside the consent wording, word for word as the terms
 *     quote it, and Save: ticked, updates go on and the consent is recorded
 *     by the service; unticked, they go off;
 *   - once on, the confirmation; and where the number replied STOP, how to
 *     get texts again.
 *
 * Shown only while the service texts updates (`GET /v1/meta`,
 * `sms_updates`), on an agreement that has been sent and is not closed, and
 * on a closed one only while updates are still on, to turn them off.
 */

/** The version of the consent wording, `smsUpdates.consent`; the service checks it (`CONSENT_VERSION` in `backend/src/notifications/sms_updates.rs`). */
export const SMS_CONSENT_VERSION = '2026-10-05'

export type SmsUpdatesApi = Pick<
  ExchangeApi,
  'meta' | 'smsUpdates' | 'setSmsUpdates' | 'requestCode' | 'addIdentifier' | 'proveIdentifier'
>

/** A piece of the consent wording: text, or one of its two addresses, shown as a link. */
export type ConsentPiece = { text: string } | { url: string }

/**
 * The consent wording in pieces, its addresses apart so that each can be a
 * link that reads exactly as the address: `https://yuppers.app/terms`, not
 * the full stop after it.
 */
export function consentPieces(consent: string): ConsentPiece[] {
  const pieces: ConsentPiece[] = []
  let at = 0
  for (const found of consent.matchAll(/https:\/\/\S+/g)) {
    const url = found[0].replace(/[.,;:]+$/, '')
    if (found.index > at) pieces.push({ text: consent.slice(at, found.index) })
    pieces.push({ url })
    at = found.index + url.length
  }
  if (at < consent.length) pieces.push({ text: consent.slice(at) })
  return pieces
}

const asked = new WeakMap<SmsUpdatesApi, Promise<boolean>>()

/** Whether the service texts updates, asked once per client and remembered. */
export function smsUpdatesOffered(api: SmsUpdatesApi): Promise<boolean> {
  let found = asked.get(api)
  if (!found) {
    found = api.meta().then(
      // Absent from a service too old to know of them.
      (meta) => (meta as Partial<typeof meta>).sms_updates === true,
      () => {
        asked.delete(api)
        return false
      },
    )
    asked.set(api, found)
  }
  return found
}

/** Where the control is. */
export type SmsUpdatesStep =
  /** Nothing to show: texts are not sent here, or not for this agreement. */
  | 'hidden'
  | 'loading'
  /** No number on the account yet: one to enter. */
  | 'addPhone'
  /** A code was texted to the number: the code to enter. */
  | 'enterCode'
  /** The box, ticked or not, with Save. */
  | 'consent'
  /** The number replied STOP: what to do. */
  | 'optedOut'

export interface SmsUpdatesControl {
  step: SmsUpdatesStep
  /** Where the service says the caller stands, once it has said. */
  standing: SmsUpdates | null
  /** The number a code was texted to, while one is awaited. */
  pending: string | null
  /**
   * The account's email address, where a code went with the one to the
   * number: adding a number takes a proof of it. Absent for an account
   * with no email address.
   */
  proofTo: string | null
  /** Whether the box is ticked. */
  checked: boolean
  setChecked(checked: boolean): void
  busy: boolean
  failure: ErrorCode | null
  /** The number typed is not a US number. */
  invalidPhone: boolean
  /**
   * Whether the box beside the number to add is ticked. No code is asked
   * for until it is; it starts unticked, and is unticked again when the
   * number is to be changed.
   */
  codeConsent: boolean
  setCodeConsent(checked: boolean): void
  /** A number was just added to the account. */
  phoneAdded: string | null
  /** What was just saved, for the screen to say. */
  saved: 'on' | 'off' | null
  requestCode(input: string): Promise<void>
  /** The code texted to the number, and the one sent to `proofTo`. */
  verify(code: string, proofCode?: string): Promise<void>
  /** Back from the code to the number. */
  changePhone(): void
  save(): Promise<void>
}

/**
 * The "Text updates" control for one agreement. `language` is the one the
 * consent wording is shown in; `onAccount` is told when a number is added,
 * so the client's view of the account follows; `email` is the account's
 * own, which adding a number takes a code to.
 */
export function useSmsUpdates(
  api: SmsUpdatesApi,
  exchange: Pick<ExchangeView, 'id' | 'state'>,
  language: string,
  onAccount?: (account: Account) => void,
  email: string | null = null,
): SmsUpdatesControl {
  const [offered, setOffered] = useState<boolean | null>(null)
  const [standing, setStanding] = useState<SmsUpdates | null>(null)
  const [pending, setPending] = useState<string | null>(null)
  const [checked, setChecked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [invalidPhone, setInvalidPhone] = useState(false)
  const [codeConsent, setCodeConsent] = useState(false)
  const [phoneAdded, setPhoneAdded] = useState<string | null>(null)
  const [saved, setSaved] = useState<'on' | 'off' | null>(null)
  // The proof from the email's code, kept should the number's code be wrong.
  const [proof, setProof] = useState<string | null>(null)
  const working = useRef(false)
  const { id, state } = exchange
  const sent = state === 'NEGOTIATING' || state === 'ACTIVE'

  useEffect(() => {
    let cancelled = false
    void smsUpdatesOffered(api).then((found) => {
      if (!cancelled) setOffered(found)
    })
    return () => {
      cancelled = true
    }
  }, [api])

  useEffect(() => {
    if (!offered || state === 'DRAFT') return
    let cancelled = false
    api.smsUpdates(id).then(
      (found) => {
        if (cancelled) return
        setStanding(found)
        setChecked(found.on)
      },
      () => {},
    )
    return () => {
      cancelled = true
    }
  }, [api, id, offered, state])

  const run = useCallback(async (work: () => Promise<void>) => {
    if (working.current) return
    working.current = true
    setBusy(true)
    setFailure(null)
    try {
      await work()
    } catch (error) {
      setFailure(failureCode(error))
    } finally {
      working.current = false
      setBusy(false)
    }
  }, [])

  const requestCode = useCallback(
    async (input: string) => {
      if (!codeConsent) return
      const phone = usPhone(input)
      setInvalidPhone(phone === null)
      if (phone === null) return
      await run(async () => {
        await api.requestCode(phone, smsCodeConsent(language))
        if (email && !proof) await api.requestCode(email)
        setPending(phone)
      })
    },
    [api, codeConsent, email, language, proof, run],
  )

  const verify = useCallback(
    async (code: string, proofCode?: string) => {
      if (pending === null) return
      await run(async () => {
        let proved = proof
        if (email && !proved) {
          proved = (await api.proveIdentifier('EMAIL', (proofCode ?? '').trim())).proof
          setProof(proved)
        }
        const account = await api.addIdentifier(pending, code.trim(), proved ?? undefined)
        onAccount?.(account)
        setPhoneAdded(pending)
        setPending(null)
        setProof(null)
        setStanding(await api.smsUpdates(id))
      })
    },
    [api, email, id, pending, onAccount, proof, run],
  )

  const changePhone = useCallback(() => {
    setPending(null)
    setFailure(null)
    setCodeConsent(false)
  }, [])

  const save = useCallback(async () => {
    await run(async () => {
      const body = checked
        ? { on: true, consent: { version: SMS_CONSENT_VERSION, language } }
        : { on: false }
      const found = await api.setSmsUpdates(id, body)
      setStanding(found)
      setChecked(found.on)
      setSaved(found.on ? 'on' : 'off')
      setPhoneAdded(null)
    })
  }, [api, id, checked, language, run])

  let step: SmsUpdatesStep
  if (offered === null) step = 'loading'
  else if (!offered || state === 'DRAFT') step = 'hidden'
  else if (standing === null) step = 'loading'
  // A closed agreement only lets updates still on be turned off.
  else if (!standing.available && !(standing.on && !sent)) step = 'hidden'
  else if (standing.opted_out) step = 'optedOut'
  else if (standing.phone == null) step = pending === null ? 'addPhone' : 'enterCode'
  else step = 'consent'

  return {
    step,
    standing,
    pending,
    proofTo: email && !proof ? email : null,
    checked,
    setChecked: (value: boolean) => {
      setChecked(value)
      setSaved(null)
    },
    busy,
    failure,
    invalidPhone,
    codeConsent,
    setCodeConsent,
    phoneAdded,
    saved,
    requestCode,
    verify,
    changePhone,
    save,
  }
}
