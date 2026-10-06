import type { Account, ErrorCode } from '@yuppers/api-client'
import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'

import { failureCode, type CodeChannel, type DeletionPreview, type SmsCodeConsent } from './api'
import type { SignInChannels } from './sign-in'
import { smsCodeConsent, useSmsCodeConsentBox, type SmsCodeConsentBox } from './sms-code-consent'

/*
 * Deleting the account, as both apps do it (DESIGN.md §4.1): what will
 * happen is said first, then a one-time code proves it is the account's
 * holder asking, then one last confirmation. The service does the deleting;
 * what is here is the order of the steps and what each needs before the next.
 */

/** Somewhere the code that confirms a deletion can go: one of the account's own identifiers. */
export interface CodeDestination {
  channel: CodeChannel
  identifier: string
}

/**
 * Where a code can be sent for this account, email first. The phone number
 * is left out where the service has said it cannot text it (`channels`) and
 * there is an email address to use instead; an account with only a phone
 * number keeps it, the one way there is to ask.
 */
export function codeDestinations(
  account: Pick<Account, 'email' | 'phone'>,
  channels: SignInChannels | null = null,
): CodeDestination[] {
  const found: CodeDestination[] = []
  if (account.email) found.push({ channel: 'EMAIL', identifier: account.email })
  const textable = channels === null || channels.phone || !account.email
  if (account.phone && textable) found.push({ channel: 'PHONE', identifier: account.phone })
  return found
}

/** The calls a deletion makes. */
export interface DeletionApi {
  deletionPreview(): Promise<DeletionPreview>
  requestDeletionCode(channel: CodeChannel, smsConsent?: SmsCodeConsent): Promise<void>
  deleteAccount(channel: CodeChannel, code: string): Promise<void>
}

export type DeletionOutcome =
  | { deleted: true }
  /** `retype` means the code was not accepted: back to the step that asks for it. */
  | { deleted: false; code: ErrorCode; retype: boolean }

/** Asks the service to delete the account, and says what became of it. It never throws. */
export async function deleteWithCode(
  api: Pick<DeletionApi, 'deleteAccount'>,
  channel: CodeChannel,
  code: string,
): Promise<DeletionOutcome> {
  try {
    await api.deleteAccount(channel, code.trim())
    return { deleted: true }
  } catch (error) {
    const failure = failureCode(error)
    return { deleted: false, code: failure, retype: failure === 'INVALID_CODE' }
  }
}

/**
 * - `explain`: what deleting does and does not do, and where the code will go.
 * - `code`: the code has been sent and is asked for.
 * - `confirm`: the last question before the account is deleted.
 */
export type DeletionStep = 'explain' | 'code' | 'confirm'

export interface AccountDeletion {
  step: DeletionStep
  /** What would happen to the account's exchanges. `null` until the service has said. */
  preview: DeletionPreview | null
  /** Set when the service could not be asked what would happen. */
  previewFailure: ErrorCode | null
  destinations: CodeDestination[]
  /** Where the code goes, or went. */
  destination: CodeDestination | null
  code: string
  /** Asked to go on without a code. */
  codeMissing: boolean
  busy: boolean
  /** Why the last request was refused, until the next one is made. */
  failure: ErrorCode | null
  /** A second code was just sent. */
  resent: boolean
  /**
   * The box beside the phone number, shown while the code is to go by text
   * (`sms-code-consent.ts`). Until it is ticked, no code is asked for.
   */
  codeConsent: SmsCodeConsentBox
  loadPreview(): void
  choose(channel: CodeChannel): void
  setCode(code: string): void
  /** Sends a code and moves on to asking for it; again, from there, sends another. */
  sendCode(): Promise<void>
  /** On to the last question, once a code has been typed. */
  review(): void
  /** One step back. */
  back(): void
  /** Deletes the account. `onDeleted` runs once the service has done it. */
  confirm(): Promise<void>
}

/**
 * The steps of deleting the signed-in account. `onDeleted` is where a client
 * forgets the session it held and goes back to its signed-out screen.
 * `language` is the one the screen is in, which the box beside a phone
 * number is shown in.
 */
export function useAccountDeletion(
  api: DeletionApi,
  account: Pick<Account, 'email' | 'phone'>,
  onDeleted: () => void | Promise<void>,
  channels: SignInChannels | null = null,
  language = 'en',
): AccountDeletion {
  const { email, phone } = account
  const destinations = codeDestinations({ email, phone }, channels)
  const [step, setStep] = useState<DeletionStep>('explain')
  const [preview, setPreview] = useState<DeletionPreview | null>(null)
  const [previewFailure, setPreviewFailure] = useState<ErrorCode | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [channel, setChannel] = useState<CodeChannel | null>(null)
  const [code, setCode] = useState('')
  const [codeMissing, setCodeMissing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [resent, setResent] = useState(false)

  const destination =
    destinations.find((found) => found.channel === channel) ?? destinations[0] ?? null
  const codeConsent = useSmsCodeConsentBox(
    destination?.channel === 'PHONE' ? destination.identifier : null,
  )

  useEffect(() => {
    let cancelled = false
    api.deletionPreview().then(
      (found) => {
        if (cancelled) return
        setPreview(found)
        setPreviewFailure(null)
      },
      (error: unknown) => {
        if (!cancelled) setPreviewFailure(failureCode(error))
      },
    )
    return () => {
      cancelled = true
    }
  }, [api, attempt])

  const loadPreview = useCallback(() => {
    setPreviewFailure(null)
    setAttempt((count) => count + 1)
  }, [])

  async function sendCode() {
    if (!destination || codeConsent.missing) return
    const again = step === 'code'
    setBusy(true)
    setFailure(null)
    setResent(false)
    try {
      await api.requestDeletionCode(
        destination.channel,
        codeConsent.checked ? smsCodeConsent(language) : undefined,
      )
      // The field starts empty for the code just sent.
      setCode('')
      setCodeMissing(false)
      setStep('code')
      setResent(again)
    } catch (error) {
      setFailure(failureCode(error))
    } finally {
      setBusy(false)
    }
  }

  function review() {
    setFailure(null)
    setResent(false)
    if (code.trim() === '') {
      setCodeMissing(true)
      return
    }
    setCodeMissing(false)
    setStep('confirm')
  }

  function back() {
    setFailure(null)
    setResent(false)
    setCodeMissing(false)
    // Back where the box is, it is unticked again: it is never remembered.
    if (step === 'code') codeConsent.setChecked(false)
    setStep(step === 'confirm' ? 'code' : 'explain')
  }

  async function confirm() {
    if (!destination) return
    setBusy(true)
    setFailure(null)
    const outcome = await deleteWithCode(api, destination.channel, code)
    if (outcome.deleted) {
      deletedNotice.announce()
      // The screen this runs on goes away with the account; nothing is set after.
      await onDeleted()
      return
    }
    setFailure(outcome.code)
    if (outcome.retype) setStep('code')
    setBusy(false)
  }

  return {
    step,
    preview,
    previewFailure,
    destinations,
    destination,
    code,
    codeMissing,
    busy,
    failure,
    resent,
    codeConsent,
    loadPreview,
    choose(next) {
      // Whatever was ticked was for the other one.
      codeConsent.setChecked(false)
      setChannel(next)
    },
    setCode(next) {
      setCode(next)
      if (next.trim() !== '') setCodeMissing(false)
    },
    sendCode,
    review,
    back,
    confirm,
  }
}

/*
 * After the last step the app is signed out, and the screen that did the
 * deleting is gone. That it worked still has to be said, once, on whatever
 * screen comes next. This is where that is kept: in memory only, so nothing
 * about the deleted account outlives the page or the running app.
 */
let announced = false
const listeners = new Set<() => void>()

function announce(next: boolean) {
  if (announced === next) return
  announced = next
  for (const listener of listeners) listener()
}

export const deletedNotice = {
  announce: () => announce(true),
  dismiss: () => announce(false),
  shown: () => announced,
  subscribe(listener: () => void) {
    listeners.add(listener)
    return () => {
      listeners.delete(listener)
    }
  },
}

/** Whether to say that the account was just deleted. */
export function useDeletedNotice(): boolean {
  return useSyncExternalStore(deletedNotice.subscribe, deletedNotice.shown, deletedNotice.shown)
}
