import type { ErrorCode, ExchangeView } from '@yuppers/api-client'
import { useCallback, useEffect, useState } from 'react'

import {
  failureCode,
  type ExchangeApi,
  type PaymentHandleChanges,
  type PaymentHandles,
} from './api'
import { fractionDigitsOf, fromMinorUnits } from './decimal'
import { formatPhone, usPhone } from './phone'

/*
 * Payment options (`backend/src/payments.rs`; DESIGN.md §7): the names a
 * person may save for being paid in another app, shown to the person who
 * owes them money on a yup where they choose to show them.
 *
 * Yuppers never moves money. A payment option is a link to someone else's
 * app, built here from the payee's name, the amount and a note, and opened
 * only when the payer presses it. Nothing is recorded from a link being
 * opened: the payer says "I've paid" themselves, as for any payment made
 * outside the product, and the payee confirms receiving it.
 *
 * The links are plain web addresses, which open the app where it is
 * installed (each app's domain claims these paths for its app: universal
 * links on iOS, app links on Android) and the app's website otherwise. No
 * custom URL scheme is used: none is documented for these apps, and a web
 * address needs no list of schemes on iOS (`LSApplicationQueriesSchemes`).
 *
 *   Venmo     https://venmo.com/u/<username>?txn=pay&amount=<amount>&note=<note>
 *             venmo.com claims `/u/*` for the app; `amount` and `note` are
 *             Venmo's long-standing payment-link parameters, no longer
 *             documented, so the sheet always asks the payer to check them.
 *   Cash App  https://cash.app/$<cashtag>/<amount>
 *             cash.app claims `/$*`; the amount in the path is widely used
 *             but not documented. Cash App takes no note in a link.
 *   PayPal    https://www.paypal.me/<name>/<amount>USD
 *             PayPal documents the amount and currency at the end of a
 *             PayPal.Me link; www.paypal.me claims every path for the app.
 *             PayPal takes no note in a link.
 *   Zelle     no links at all: the email address or number is shown, to
 *             copy into the bank's app.
 *
 * The apps work in US dollars; for any other currency no amount is put in
 * a link, and the payer is told the amount to enter.
 */

/** The apps, in the order they are offered. */
export const PAYMENT_APPS = ['venmo', 'cash_app', 'paypal', 'zelle'] as const

export type PaymentApp = (typeof PAYMENT_APPS)[number]

/** The apps a link can open: all but Zelle. */
export type LinkedApp = Exclude<PaymentApp, 'zelle'>

/** The longest note Venmo takes. */
export const NOTE_MAX_CHARS = 280

/** The longest item title put in a note, so that the yup's code always fits. */
const NOTE_TITLE_MAX_CHARS = 60

/** A Venmo username as stored: 5 to 30 letters, digits, `-` or `_`, without its `@`. */
export function venmoUsername(input: string): string | null {
  const trimmed = input.trim()
  const name = trimmed.startsWith('@') ? trimmed.slice(1) : trimmed
  return /^[A-Za-z0-9_-]{5,30}$/.test(name) ? name : null
}

/** A $Cashtag as stored: 1 to 20 letters, digits or `_`, one a letter, without its `$`. */
export function cashtag(input: string): string | null {
  const trimmed = input.trim()
  const tag = trimmed.startsWith('$') ? trimmed.slice(1) : trimmed
  return /^[A-Za-z0-9_]{1,20}$/.test(tag) && /[A-Za-z]/.test(tag) ? tag : null
}

/** A PayPal.Me name as stored: 1 to 20 letters or digits, typed alone or as the link. */
export function paypalName(input: string): string | null {
  const name = input
    .trim()
    .replace(/^(https?:\/\/)?(www\.)?paypal\.me\//i, '')
    .replace(/\/+$/, '')
  return /^[A-Za-z0-9]{1,20}$/.test(name) ? name : null
}

/**
 * Where Zelle pays someone, as stored: a lower-case email address, or a US
 * number as `+1` and ten digits. The service also refuses a number
 * elsewhere in the North American plan, such as Canada's.
 */
export function zelleRecipient(input: string): string | null {
  const text = input.trim()
  if (text.includes('@')) {
    const email = text.toLowerCase()
    const at = email.lastIndexOf('@')
    const local = email.slice(0, at)
    const domain = email.slice(at + 1)
    const valid =
      /^[\x21-\x7e]+$/.test(email) &&
      email.length <= 254 &&
      local.length > 0 &&
      !local.includes('@') &&
      domain.includes('.') &&
      domain.split('.').every((label) => label.length > 0)
    return valid ? email : null
  }
  return usPhone(text)
}

const NORMALIZE: Record<PaymentApp, (input: string) => string | null> = {
  venmo: venmoUsername,
  cash_app: cashtag,
  paypal: paypalName,
  zelle: zelleRecipient,
}

/** One payment option as stored, `null` for one that is not one, `''` for none. */
export function normalizeHandle(app: PaymentApp, input: string): string | null {
  return input.trim() === '' ? '' : NORMALIZE[app](input)
}

/**
 * A saved option as its owner sees it, in the list and in its field: as
 * stored, with a US number for Zelle written `(202) 555-0142`. `''` for one
 * not saved.
 */
export function handleShown(app: PaymentApp, saved: PaymentHandles | null | undefined): string {
  const value = saved?.[app]
  if (!value) return ''
  return app === 'zelle' ? zelleShown(value) : value
}

/** The apps with an option saved, in the apps' order. */
export function addedApps(saved: PaymentHandles | null | undefined): PaymentApp[] {
  return PAYMENT_APPS.filter((app) => Boolean(saved?.[app]))
}

/** The apps that can still be added: those with nothing saved. */
export function appsToAdd(saved: PaymentHandles | null | undefined): PaymentApp[] {
  return PAYMENT_APPS.filter((app) => !saved?.[app])
}

/**
 * The account row's summary: the names of the apps added, such as
 * "Venmo, Zelle", or `none` with none added.
 */
export function paymentOptionsSummary(
  saved: PaymentHandles | null | undefined,
  names: Record<PaymentApp, string>,
  none: string,
): string {
  const added = addedApps(saved)
  return added.length === 0 ? none : added.map((app) => names[app]).join(', ')
}

export function hasAnyHandle(handles: PaymentHandles | null | undefined): boolean {
  return PAYMENT_APPS.some((app) => Boolean(handles?.[app]))
}

/** A Zelle recipient as shown: a US number as `(202) 555-0142`, an email address as it is. */
export function zelleShown(recipient: string): string {
  return formatPhone(recipient)
}

/**
 * An amount as a payment link carries it: US dollars only, as a plain
 * decimal without a currency sign or grouping, and whole dollars without
 * cents (`100`, `100.5` written `100.50`). `null` for any other currency,
 * which no link here can carry.
 */
export function linkAmount(minor: number, currency: string): string | null {
  if (currency !== 'USD' || !Number.isSafeInteger(minor) || minor <= 0) return null
  const plain = fromMinorUnits(minor, fractionDigitsOf(currency))
  return plain.endsWith('.00') ? plain.slice(0, -3) : plain
}

/**
 * The note for a payment: the item's title and the yup's reference, from
 * `payments.note`, and nothing else from the agreement. A long title is
 * cut short, so that the reference always fits Venmo's limit.
 */
export function paymentNote(template: string, title: string, code: string): string {
  const oneLine = title.replace(/\s+/g, ' ').trim()
  const chars = [...oneLine]
  const short =
    chars.length > NOTE_TITLE_MAX_CHARS
      ? `${chars.slice(0, NOTE_TITLE_MAX_CHARS - 1).join('').trimEnd()}…`
      : oneLine
  const note = template.replace('{title}', short).replace('{code}', code)
  return [...note].slice(0, NOTE_MAX_CHARS).join('')
}

/** A Venmo payment link, with the amount and note where given. */
export function venmoUrl(username: string, amount: string | null, note: string | null): string {
  const query = new URLSearchParams({ txn: 'pay' })
  if (amount) query.set('amount', amount)
  if (note) query.set('note', note)
  // URLSearchParams writes a space as `+`; `%20` reads the same everywhere.
  return `https://venmo.com/u/${encodeURIComponent(username)}?${query.toString().replace(/\+/g, '%20')}`
}

/** A Cash App link, with the amount where given. Cash App takes no note. */
export function cashAppUrl(tag: string, amount: string | null): string {
  const base = `https://cash.app/$${encodeURIComponent(tag)}`
  return amount ? `${base}/${amount}` : base
}

/** A PayPal.Me link, with the amount in US dollars where given. PayPal takes no note. */
export function paypalUrl(name: string, amount: string | null): string {
  const base = `https://www.paypal.me/${encodeURIComponent(name)}`
  return amount ? `${base}/${amount}USD` : base
}

/** One way to pay, as the payer's sheet offers it. */
export type PayOption =
  | {
      app: LinkedApp
      handle: string
      url: string
      /** Whether the link fills in the amount; if not, the payer enters it. */
      amountFilled: boolean
      /** Whether the link fills in the note too. */
      noteFilled: boolean
      /** When the payee changed it, RFC 3339, if recently enough to warn the payer. */
      changedAt: string | null
    }
  | { app: 'zelle'; handle: string; shown: string; changedAt: string | null }

/** The payee's options, in the apps' order, each with its link. */
export function payOptions(
  handles: PaymentHandles | null | undefined,
  amountMinor: number,
  currency: string,
  note: string,
  changes?: PaymentHandleChanges | null,
): PayOption[] {
  if (!handles) return []
  const changedAt = (app: PaymentApp) => changes?.[app] ?? null
  const amount = linkAmount(amountMinor, currency)
  const options: PayOption[] = []
  if (handles.venmo) {
    options.push({
      app: 'venmo',
      handle: handles.venmo,
      url: venmoUrl(handles.venmo, amount, note),
      amountFilled: amount !== null,
      noteFilled: true,
      changedAt: changedAt('venmo'),
    })
  }
  if (handles.cash_app) {
    options.push({
      app: 'cash_app',
      handle: handles.cash_app,
      url: cashAppUrl(handles.cash_app, amount),
      amountFilled: amount !== null,
      noteFilled: false,
      changedAt: changedAt('cash_app'),
    })
  }
  if (handles.paypal) {
    options.push({
      app: 'paypal',
      handle: handles.paypal,
      url: paypalUrl(handles.paypal, amount),
      amountFilled: amount !== null,
      noteFilled: false,
      changedAt: changedAt('paypal'),
    })
  }
  if (handles.zelle) {
    options.push({
      app: 'zelle',
      handle: handles.zelle,
      shown: zelleShown(handles.zelle),
      changedAt: changedAt('zelle'),
    })
  }
  return options
}

type Contribution = NonNullable<ExchangeView['in_force_revision']>['terms']['contributions'][number]

/**
 * Whether to offer the payer "Pay": a money item they owe on the agreement
 * in force, not yet marked paid (or disputed, which puts it back), while
 * the payee shows their payment options to them.
 */
export function payOffered(
  exchange: Pick<ExchangeView, 'state' | 'you' | 'payment_options'>,
  contribution: Pick<Contribution, 'type' | 'from' | 'amount_minor'>,
  status: string,
): boolean {
  return (
    exchange.state === 'ACTIVE' &&
    contribution.type === 'MONEY' &&
    contribution.from === exchange.you &&
    (contribution.amount_minor ?? 0) > 0 &&
    (status === 'PENDING' || status === 'DISPUTED') &&
    hasAnyHandle(exchange.payment_options?.theirs)
  )
}

/**
 * Whether to offer a party the choice to show their payment options: they
 * receive money in the terms on the table or in force, on a yup that has
 * been sent and is not closed. One already showing them can always stop.
 */
export function showOffered(
  exchange: Pick<ExchangeView, 'state' | 'you' | 'open_revision' | 'in_force_revision' | 'payment_options'>,
): boolean {
  if (exchange.payment_options?.shown) return true
  if (exchange.state !== 'NEGOTIATING' && exchange.state !== 'ACTIVE') return false
  return [exchange.open_revision, exchange.in_force_revision].some((revision) =>
    revision?.terms.contributions.some(
      (contribution) => contribution.type === 'MONEY' && contribution.from !== exchange.you,
    ),
  )
}

/**
 * What of an exchange's view the payer's screen must notice change, besides
 * its version: whether the other party shows payment options, and which.
 * Showing them changes no version, so a screen that refreshes only on a new
 * version would keep showing ones that were turned off.
 */
export function paymentOptionsKey(exchange: Pick<ExchangeView, 'payment_options'>): string {
  return JSON.stringify(exchange.payment_options ?? null)
}

export type PaymentHandlesApi = Pick<
  ExchangeApi,
  'paymentHandles' | 'setPaymentHandle' | 'removePaymentHandle'
>

/**
 * Where the payment options screen is: the list of those added, choosing
 * the app to add, or the one field for adding or editing an app's option.
 */
export type PaymentOptionsStep =
  | { name: 'list' }
  | { name: 'pick' }
  | { name: 'field'; app: PaymentApp; adding: boolean }

/** What last happened on the screen, to say so and to put the focus back. */
export type PaymentOptionsDone =
  | { what: 'saved'; app: PaymentApp }
  | { what: 'removed'; app: PaymentApp; last: boolean }
  /** Adding or editing was cancelled; the focus goes back where it came from. */
  | { what: 'cancelled'; app: PaymentApp | null }

/** The payment options screen (`usePaymentOptions`). */
export interface PaymentOptionsScreen {
  /** What is saved, once loaded; `null` until then. */
  saved: PaymentHandles | null
  /** Why it could not be loaded, if it could not. */
  loadFailure: ErrorCode | null
  step: PaymentOptionsStep
  /** The field's text, while adding or editing. */
  input: string
  setInput(value: string): void
  /** Whether the field's text was found not to be one, once saving was tried. */
  invalid: boolean
  busy: boolean
  /** A refusal from the service, for what was last tried. */
  failure: ErrorCode | null
  done: PaymentOptionsDone | null
  /** The app whose removal waits to be confirmed. */
  confirming: PaymentApp | null
  /** "Add a payment option": choosing the app. */
  startAdding(): void
  /** An app chosen: its field, empty. */
  pick(app: PaymentApp): void
  /** "Edit": the app's field, filled in with what is saved. */
  edit(app: PaymentApp): void
  /** Back to the list without saving. */
  cancel(): void
  save(): Promise<boolean>
  /** "Remove": asks first. */
  askToRemove(app: PaymentApp): void
  /** Not removing it after all. */
  keep(): void
  /** Removing it, once confirmed. */
  remove(): Promise<boolean>
  /** Loading again, after it failed. */
  reload(): void
}

/**
 * The payment options screen, the same on both apps: the options added,
 * each added, changed or removed on its own (`PUT`, `DELETE
 * /v1/me/payment-handles/{kind}`), so that one is never saved or lost by
 * accident with another. Removing the last one stops showing them on every
 * yup; the confirmation says so.
 */
export function usePaymentOptions(api: PaymentHandlesApi): PaymentOptionsScreen {
  const [saved, setSaved] = useState<PaymentHandles | null>(null)
  const [loadFailure, setLoadFailure] = useState<ErrorCode | null>(null)
  const [loads, setLoads] = useState(0)
  const [step, setStep] = useState<PaymentOptionsStep>({ name: 'list' })
  const [input, setInputText] = useState('')
  const [invalid, setInvalid] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [done, setDone] = useState<PaymentOptionsDone | null>(null)
  const [confirming, setConfirming] = useState<PaymentApp | null>(null)

  useEffect(() => {
    let cancelled = false
    setLoadFailure(null)
    api.paymentHandles().then(
      (found) => {
        if (!cancelled) setSaved(found)
      },
      (error: unknown) => {
        if (!cancelled) setLoadFailure(failureCode(error))
      },
    )
    return () => {
      cancelled = true
    }
  }, [api, loads])

  const reload = useCallback(() => setLoads((count) => count + 1), [])

  const setInput = useCallback((value: string) => {
    setInputText(value)
    setInvalid(false)
  }, [])

  const open = useCallback((next: PaymentOptionsStep, text = '') => {
    setStep(next)
    setInputText(text)
    setInvalid(false)
    setFailure(null)
    setDone(null)
    setConfirming(null)
  }, [])

  const startAdding = useCallback(() => open({ name: 'pick' }), [open])

  const pick = useCallback(
    (app: PaymentApp) => open({ name: 'field', app, adding: true }),
    [open],
  )

  const edit = useCallback(
    (app: PaymentApp) => open({ name: 'field', app, adding: false }, handleShown(app, saved)),
    [open, saved],
  )

  const cancel = useCallback(() => {
    const app = step.name === 'field' && !step.adding ? step.app : null
    open({ name: 'list' })
    setDone({ what: 'cancelled', app })
  }, [open, step])

  const save = useCallback(async () => {
    if (step.name !== 'field') return false
    const value = normalizeHandle(step.app, input)
    // An empty field is not one either: an option is removed with "Remove".
    if (!value) {
      setInvalid(true)
      return false
    }
    setBusy(true)
    setFailure(null)
    try {
      const stored = await api.setPaymentHandle(step.app, value)
      setSaved(stored)
      setStep({ name: 'list' })
      setInputText('')
      setDone({ what: 'saved', app: step.app })
      return true
    } catch (error) {
      setFailure(failureCode(error))
      return false
    } finally {
      setBusy(false)
    }
  }, [api, input, step])

  const askToRemove = useCallback((app: PaymentApp) => {
    setFailure(null)
    setDone(null)
    setConfirming(app)
  }, [])

  const keep = useCallback(() => setConfirming(null), [])

  const remove = useCallback(async () => {
    const app = confirming
    if (!app) return false
    setBusy(true)
    setFailure(null)
    try {
      const left = await api.removePaymentHandle(app)
      setSaved(left)
      setDone({ what: 'removed', app, last: !hasAnyHandle(left) })
      return true
    } catch (error) {
      setFailure(failureCode(error))
      return false
    } finally {
      setConfirming(null)
      setBusy(false)
    }
  }, [api, confirming])

  return {
    saved,
    loadFailure,
    step,
    input,
    setInput,
    invalid,
    busy,
    failure,
    done,
    confirming,
    startAdding,
    pick,
    edit,
    cancel,
    save,
    askToRemove,
    keep,
    remove,
    reload,
  }
}

/**
 * The account's saved payment options, for the account screen's summary
 * and a yup's box: `undefined` until known, `null` if they cannot be known.
 * A new `refresh` reads them again, for a screen shown again after they
 * may have changed elsewhere.
 */
export function useSavedPaymentHandles(
  api: Pick<ExchangeApi, 'paymentHandles'>,
  refresh = 0,
): PaymentHandles | null | undefined {
  const [saved, setSaved] = useState<PaymentHandles | null | undefined>(undefined)
  useEffect(() => {
    let cancelled = false
    api.paymentHandles().then(
      (found) => {
        if (!cancelled) setSaved(found)
      },
      () => {
        if (!cancelled) setSaved(null)
      },
    )
    return () => {
      cancelled = true
    }
  }, [api, refresh])
  return saved
}

/** Whether the account has any payment options saved; `null` until known, `false` if it cannot be known. */
export function useHasPaymentHandles(
  api: Pick<ExchangeApi, 'paymentHandles'>,
  refresh = 0,
): boolean | null {
  const saved = useSavedPaymentHandles(api, refresh)
  return saved === undefined ? null : hasAnyHandle(saved)
}

/**
 * Whether the person receives money in these terms, so that showing their
 * payment options means something: for the box beside signing or sending.
 */
export function receivesMoney(
  terms: { contributions: readonly { type: string; from: string }[] } | null | undefined,
  you: string,
): boolean {
  return Boolean(
    terms?.contributions.some(
      (contribution) => contribution.type === 'MONEY' && contribution.from !== you,
    ),
  )
}

export type PaymentOptionsApi = Pick<ExchangeApi, 'paymentHandles' | 'setPaymentOptions'>

/** The control on a yup: whether this party shows their payment options. */
export interface ShowPaymentOptions {
  /** Whether the account has any saved; `null` until known. */
  hasHandles: boolean | null
  shown: boolean
  busy: boolean
  failure: ErrorCode | null
  /** What was last done here, to say so. */
  changed: 'shown' | 'hidden' | null
  set(on: boolean): Promise<boolean>
}

/**
 * Showing payment options on a yup, or not. `onChanged` is told once the
 * service has done it, to bring the exchange up to date.
 */
export function useShowPaymentOptions(
  api: PaymentOptionsApi,
  exchange: Pick<ExchangeView, 'id' | 'payment_options'>,
  onChanged: () => void,
  /** A new value reads the saved options again, as `useSavedPaymentHandles`. */
  refresh = 0,
): ShowPaymentOptions {
  const hasHandles = useHasPaymentHandles(api, refresh)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [changed, setChanged] = useState<'shown' | 'hidden' | null>(null)
  const [shown, setShown] = useState(Boolean(exchange.payment_options?.shown))
  const fromService = Boolean(exchange.payment_options?.shown)

  useEffect(() => setShown(fromService), [fromService])

  const set = useCallback(
    async (on: boolean) => {
      setBusy(true)
      setFailure(null)
      setChanged(null)
      // The box follows the hand at once, and goes back if the service refuses.
      const before = shown
      setShown(on)
      try {
        const done = await api.setPaymentOptions(exchange.id, on)
        setShown(done.on)
        setChanged(done.on ? 'shown' : 'hidden')
        onChanged()
        return true
      } catch (error) {
        setShown(before)
        setFailure(failureCode(error))
        return false
      } finally {
        setBusy(false)
      }
    },
    [api, exchange.id, onChanged, shown],
  )

  return { hasHandles, shown, busy, failure, changed, set }
}
