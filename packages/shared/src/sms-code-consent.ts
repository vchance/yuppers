import type { components } from '@yuppers/api-client'
import { useState } from 'react'

import type { Wording } from './wording/types'

/*
 * Consent to a one-time code by text: "Yuppers.app sign-in codes" (README,
 * "Signing in"). Every form that texts a code shows, beside the number, a
 * box that is never ticked to begin with and never remembered, with words
 * that say what will be sent, how often, the rates, HELP and STOP, and the
 * two policies' addresses (`smsCode` in the wording files, quoted word for
 * word by the terms). Until it is ticked, the form's button stays disabled
 * and says why. The request then names the wording shown (`sms_consent`),
 * which the service checks and records; without it a code for a phone
 * number is refused (`SMS_CONSENT_REQUIRED`). Email addresses need none.
 *
 * The three forms: signing in, as soon as what is typed reads as a phone
 * number; confirming a deletion by text; and checking a number added for
 * agreement updates. Each has its own words before the shared rest.
 */

export type SmsCodeConsent = components['schemas']['SmsCodeConsent']

/** The version of the wording beside the box, `smsCode`; the service checks it (`CODE_CONSENT_VERSION` in `backend/src/code_consent.rs`). */
export const SMS_CODE_CONSENT_VERSION = '2026-10-06'

/** Which form the box is on, and so which words it shows. */
export type SmsCodePurpose = 'signIn' | 'deleteAccount' | 'verifyNumber'

/** The words beside the box on a form. */
export function smsCodeConsentLabel(w: Wording['smsCode'], purpose: SmsCodePurpose): string {
  return w[purpose]
}

/** What a request for a code by text says once the box is ticked: the wording's version, and the language it was shown in. */
export function smsCodeConsent(language: string): SmsCodeConsent {
  return { version: SMS_CODE_CONSENT_VERSION, language }
}

/**
 * Whether what someone typed reads as a phone number, so that a code for it
 * would go by text: no `@`, and nothing but digits and the signs a number is
 * written with, with at least one digit. The box appears from the first
 * digit, before the number is whole, so that it is there to read while the
 * rest is typed.
 */
export function readsAsPhone(input: string): boolean {
  const typed = input.trim()
  return /\d/.test(typed) && /^\+?[\d\s().-]+$/.test(typed)
}

/** The box on one form: shown or not, ticked or not. */
export interface SmsCodeConsentBox {
  /** Whether the form shows the box: a code would go by text. */
  shown: boolean
  /** Whether it is ticked, for what it is shown for now. */
  checked: boolean
  setChecked(checked: boolean): void
  /** Whether the form's button must wait for the box. */
  missing: boolean
}

/**
 * The box for a code to `target`, the number as typed, or `null` where no
 * code would go by text. It starts unticked, and is unticked again whenever
 * `target` changes: what was ticked was for that number. Nothing about it
 * is kept anywhere but here.
 */
export function useSmsCodeConsentBox(target: string | null): SmsCodeConsentBox {
  const [ticked, setTicked] = useState<{ target: string | null; checked: boolean }>({
    target,
    checked: false,
  })
  // A new number unticks it for good, even if the old one is typed again.
  if (ticked.target !== target) setTicked({ target, checked: false })
  const shown = target !== null
  const checked = shown && ticked.target === target && ticked.checked
  return {
    shown,
    checked,
    setChecked: (value: boolean) => setTicked({ target, checked: value }),
    missing: shown && !checked,
  }
}
