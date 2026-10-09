import type { Account, ErrorCode, ExchangeView } from '@yuppers/api-client'
import { useCallback, useRef, useState } from 'react'

import {
  combineOffer,
  failureCode,
  type BoundAddress,
  type CombineOffer,
  type ExchangeApi,
} from './api'
import { usPhone } from './phone'
import { smsCodeConsent, useSmsCodeConsentBox, type SmsCodeConsentBox } from './sms-code-consent'

/*
 * An invitation sent to an email address or phone number the signed-in
 * account does not have (`sent_to` in its preview), as both apps handle it:
 * "This invitation was sent to an email address that isn't on your
 * account. Type that address to add it and open the invitation." Only its
 * kind is ever said; the person types the address, and the service sends a
 * code only if it is the one. Once the code is in:
 *
 *   - an address nobody has is added and the invitation opens, named, so
 *     nobody has to confirm the person;
 *   - an address on another of the person's accounts brings the offer to
 *     combine the two, and once combined, the invitation opens;
 *   - an account with another address of that kind is told first that
 *     adding this one replaces it (`replaces`), which takes a code to one of
 *     its own first, as changing one does; it can sign in with the invited
 *     address instead.
 */

export type InvitationAddressApi = Pick<
  ExchangeApi,
  | 'requestInvitationAddressCode'
  | 'addInvitationAddress'
  | 'combineAccounts'
  | 'claimInvitation'
  | 'requestCode'
  | 'proveIdentifier'
>

export interface InvitationAddressControl {
  /**
   * `prove`: where the code proving one of the account's own goes, and
   * `proveCode` once it was sent (only where adding replaces one); `offer`:
   * the address to type; `code`: the code sent to it.
   */
  step: 'prove' | 'proveCode' | 'offer' | 'code'
  /** While proving: where the code goes, one of the account's own. */
  proveTo: string | null
  /** The account's other one, to send that code there instead. */
  proveAlternative: string | null
  proveElsewhere(): void
  /** What is typed as the address the invitation was sent to. */
  input: string
  setInput(input: string): void
  /** The number typed is not a US number. */
  invalidPhone: boolean
  /** Where the code went, as the service takes it. */
  typed: string | null
  code: string
  setCode(code: string): void
  /** The box beside a number a code would be texted to. */
  codeConsent: SmsCodeConsentBox
  busy: boolean
  failure: ErrorCode | null
  /** The offer to combine, once the code showed the address is another account's. */
  offer: CombineOffer | null
  sendCode(): Promise<void>
  confirm(): Promise<void>
  combine(): Promise<void>
  dismissOffer(): void
}

/**
 * The steps for `sentTo`, the address the invitation `token` was sent to,
 * for `account`, the one signed in. `onOpened` is given the exchange once
 * the invitation is the account's; `onAccount` the account once it changed.
 */
export function useInvitationAddress(
  api: InvitationAddressApi,
  token: string,
  sentTo: BoundAddress,
  account: Pick<Account, 'email' | 'phone'>,
  language: string,
  onOpened: (exchange: ExchangeView) => void,
  onAccount: (account: Account) => void,
): InvitationAddressControl {
  const phone = sentTo.kind === 'PHONE'
  // Replacing one: the code goes first to the one replaced.
  const replaced = (phone ? account.phone : account.email) ?? null
  const [step, setStep] = useState<InvitationAddressControl['step']>(
    sentTo.replaces && replaced ? 'prove' : 'offer',
  )
  const [proveTo, setProveTo] = useState<string | null>(sentTo.replaces ? replaced : null)
  const [proof, setProof] = useState<string | null>(null)
  const [input, setInputState] = useState('')
  const [invalidPhone, setInvalidPhone] = useState(false)
  const [typed, setTyped] = useState<string | null>(null)
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [offer, setOffer] = useState<CombineOffer | null>(null)
  const working = useRef(false)

  const textedTo =
    step === 'prove' && proveTo && !proveTo.includes('@')
      ? proveTo
      : step === 'offer' && phone
        ? (usPhone(input) ?? (input.trim() === '' ? null : input.trim()))
        : null
  const codeConsent = useSmsCodeConsentBox(textedTo)
  const proveAlternative =
    step === 'prove'
      ? ([account.email, account.phone].find((value) => value && value !== proveTo) ?? null)
      : null

  const run = useCallback(async (work: () => Promise<void>) => {
    if (working.current) return
    working.current = true
    setBusy(true)
    setFailure(null)
    try {
      await work()
    } catch (error) {
      const offered = combineOffer(error)
      if (offered) setOffer(offered)
      else setFailure(failureCode(error))
    } finally {
      working.current = false
      setBusy(false)
    }
  }, [])

  return {
    step,
    proveTo,
    proveAlternative,
    proveElsewhere() {
      if (!proveAlternative) return
      setFailure(null)
      setProveTo(proveAlternative)
    },
    input,
    setInput(value: string) {
      setInputState(value)
      setInvalidPhone(false)
    },
    invalidPhone,
    typed,
    code,
    setCode,
    codeConsent,
    busy,
    failure,
    offer,
    async sendCode() {
      if (step === 'prove' || step === 'proveCode') {
        if (!proveTo || (step === 'prove' && codeConsent.missing)) return
        const to = proveTo
        await run(async () => {
          await api.requestCode(to, to.includes('@') ? undefined : smsCodeConsent(language))
          setStep('proveCode')
          setCode('')
        })
        return
      }
      let identifier: string
      if (step === 'code' && typed) {
        identifier = typed
      } else if (phone) {
        const number = usPhone(input)
        setInvalidPhone(number === null)
        if (number === null || codeConsent.missing) return
        identifier = number
      } else {
        identifier = input.trim()
        if (!identifier) return
      }
      await run(async () => {
        await api.requestInvitationAddressCode(
          token,
          identifier,
          phone ? smsCodeConsent(language) : undefined,
        )
        setTyped(identifier)
        setStep('code')
        setCode('')
      })
    },
    async confirm() {
      if (step === 'proveCode' && proveTo) {
        const channel = proveTo.includes('@') ? 'EMAIL' : 'PHONE'
        await run(async () => {
          const proved = await api.proveIdentifier(channel, code.trim())
          setProof(proved.proof)
          setStep('offer')
          setCode('')
        })
        return
      }
      if (!typed) return
      const identifier = typed
      await run(async () => {
        onOpened(
          await api.addInvitationAddress(
            token,
            identifier,
            code.trim(),
            sentTo.replaces,
            proof ?? undefined,
          ),
        )
      })
    },
    async combine() {
      if (!offer) return
      const offered = offer
      await run(async () => {
        onAccount(
          await api.combineAccounts(
            offered.token,
            offered.proof_required ? (proof ?? undefined) : undefined,
          ),
        )
        setOffer(null)
        onOpened(await api.claimInvitation(token))
      })
    },
    dismissOffer() {
      setOffer(null)
      setStep('offer')
      setFailure(null)
    },
  }
}
