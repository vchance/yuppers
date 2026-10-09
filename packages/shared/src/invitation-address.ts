import type { Account, ErrorCode, ExchangeView } from '@yuppers/api-client'
import { useCallback, useRef, useState } from 'react'

import {
  combineOffer,
  failureCode,
  type BoundAddress,
  type CombineOffer,
  type ExchangeApi,
} from './api'
import { smsCodeConsent, useSmsCodeConsentBox, type SmsCodeConsentBox } from './sms-code-consent'

/*
 * An invitation sent to an email address or phone number the signed-in
 * account does not have (`sent_to` in its preview), as both apps handle it:
 * "This invitation was sent to j•••@gmail.com. Add it to your account to
 * open it." The address is only ever shown masked; the service sends the
 * code there itself. Once the code is in:
 *
 *   - an address nobody has is added and the invitation opens, named, so
 *     nobody has to confirm the person;
 *   - an address on another of the person's accounts brings the offer to
 *     combine the two, and once combined, the invitation opens;
 *   - an account with another address of that kind is told first that
 *     adding this one replaces it (`replaces`), and can sign in with the
 *     invited address instead.
 */

export type InvitationAddressApi = Pick<
  ExchangeApi,
  'requestInvitationAddressCode' | 'addInvitationAddress' | 'combineAccounts' | 'claimInvitation'
>

export interface InvitationAddressControl {
  step: 'offer' | 'code'
  code: string
  setCode(code: string): void
  /** The box beside a number the code would be texted to. */
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
 * The steps for `sentTo`, the address the invitation `token` names.
 * `onOpened` is given the exchange once the invitation is the account's;
 * `onAccount` the account once it changed.
 */
export function useInvitationAddress(
  api: InvitationAddressApi,
  token: string,
  sentTo: BoundAddress,
  language: string,
  onOpened: (exchange: ExchangeView) => void,
  onAccount: (account: Account) => void,
): InvitationAddressControl {
  const [step, setStep] = useState<'offer' | 'code'>('offer')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [offer, setOffer] = useState<CombineOffer | null>(null)
  const working = useRef(false)
  const phone = sentTo.kind === 'PHONE'
  const codeConsent = useSmsCodeConsentBox(phone && step === 'offer' ? sentTo.masked : null)

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
    code,
    setCode,
    codeConsent,
    busy,
    failure,
    offer,
    async sendCode() {
      if (codeConsent.missing) return
      await run(async () => {
        await api.requestInvitationAddressCode(token, phone ? smsCodeConsent(language) : undefined)
        setStep('code')
        setCode('')
      })
    },
    async confirm() {
      await run(async () => {
        onOpened(await api.addInvitationAddress(token, code.trim(), sentTo.replaces))
      })
    },
    async combine() {
      if (!offer) return
      const offered = offer.token
      await run(async () => {
        onAccount(await api.combineAccounts(offered))
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
