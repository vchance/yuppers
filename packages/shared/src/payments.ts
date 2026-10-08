import type { ErrorCode, ExchangeView } from '@yuppers/api-client'
import { useCallback, useEffect, useState } from 'react'

import { failureCode, type ExchangeApi, type PaymentHandles } from './api'
import { fractionDigitsOf, fromMinorUnits } from './decimal'
import { usPhone } from './sms-updates'

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

/** Payment options as typed, one string per app. */
export type HandleInputs = Record<PaymentApp, string>

export const NO_HANDLE_INPUTS: HandleInputs = { venmo: '', cash_app: '', paypal: '', zelle: '' }

/** What was saved, for filling the form. */
export function handleInputs(saved: PaymentHandles | null | undefined): HandleInputs {
  return {
    venmo: saved?.venmo ?? '',
    cash_app: saved?.cash_app ?? '',
    paypal: saved?.paypal ?? '',
    zelle: saved?.zelle ? zelleShown(saved.zelle) : '',
  }
}

/**
 * The form's options as the service takes them, or the apps whose entry is
 * not one. An empty entry is no option.
 */
export function readHandles(
  inputs: HandleInputs,
): { handles: PaymentHandles; invalid: PaymentApp[] } {
  const handles: PaymentHandles = { venmo: null, cash_app: null, paypal: null, zelle: null }
  const invalid: PaymentApp[] = []
  for (const app of PAYMENT_APPS) {
    const value = normalizeHandle(app, inputs[app])
    if (value === null) invalid.push(app)
    else handles[app] = value === '' ? null : value
  }
  return { handles, invalid }
}

export function hasAnyHandle(handles: PaymentHandles | null | undefined): boolean {
  return PAYMENT_APPS.some((app) => Boolean(handles?.[app]))
}

/** A Zelle recipient as shown: a US number as `(202) 555-0142`, an email address as it is. */
export function zelleShown(recipient: string): string {
  const match = /^\+1(\d{3})(\d{3})(\d{4})$/.exec(recipient)
  return match ? `(${match[1]}) ${match[2]}-${match[3]}` : recipient
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
    }
  | { app: 'zelle'; handle: string; shown: string }

/** The payee's options, in the apps' order, each with its link. */
export function payOptions(
  handles: PaymentHandles | null | undefined,
  amountMinor: number,
  currency: string,
  note: string,
): PayOption[] {
  if (!handles) return []
  const amount = linkAmount(amountMinor, currency)
  const options: PayOption[] = []
  if (handles.venmo) {
    options.push({
      app: 'venmo',
      handle: handles.venmo,
      url: venmoUrl(handles.venmo, amount, note),
      amountFilled: amount !== null,
      noteFilled: true,
    })
  }
  if (handles.cash_app) {
    options.push({
      app: 'cash_app',
      handle: handles.cash_app,
      url: cashAppUrl(handles.cash_app, amount),
      amountFilled: amount !== null,
      noteFilled: false,
    })
  }
  if (handles.paypal) {
    options.push({
      app: 'paypal',
      handle: handles.paypal,
      url: paypalUrl(handles.paypal, amount),
      amountFilled: amount !== null,
      noteFilled: false,
    })
  }
  if (handles.zelle) {
    options.push({ app: 'zelle', handle: handles.zelle, shown: zelleShown(handles.zelle) })
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
  'paymentHandles' | 'setPaymentHandles' | 'removePaymentHandles'
>

/** The account screen's form: load, edit, save, remove. */
export interface PaymentHandlesForm {
  /** What is saved, once loaded. */
  saved: PaymentHandles | null
  inputs: HandleInputs
  set(app: PaymentApp, value: string): void
  /** The apps whose entry is not one, once a save was tried. */
  invalid: PaymentApp[]
  busy: boolean
  failure: ErrorCode | null
  /** What last happened, to say so. */
  done: 'saved' | 'removed' | null
  save(): Promise<boolean>
  remove(): Promise<boolean>
}

export function usePaymentHandles(api: PaymentHandlesApi): PaymentHandlesForm {
  const [saved, setSaved] = useState<PaymentHandles | null>(null)
  const [inputs, setInputs] = useState<HandleInputs>(NO_HANDLE_INPUTS)
  const [invalid, setInvalid] = useState<PaymentApp[]>([])
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [done, setDone] = useState<'saved' | 'removed' | null>(null)

  useEffect(() => {
    let cancelled = false
    api.paymentHandles().then(
      (found) => {
        if (cancelled) return
        setSaved(found)
        setInputs(handleInputs(found))
      },
      (error: unknown) => {
        if (!cancelled) setFailure(failureCode(error))
      },
    )
    return () => {
      cancelled = true
    }
  }, [api])

  const set = useCallback((app: PaymentApp, value: string) => {
    setInputs((previous) => ({ ...previous, [app]: value }))
    setInvalid((previous) => previous.filter((other) => other !== app))
    setDone(null)
  }, [])

  const save = useCallback(async () => {
    const read = readHandles(inputs)
    setInvalid(read.invalid)
    setDone(null)
    if (read.invalid.length > 0) return false
    setBusy(true)
    setFailure(null)
    try {
      const stored = await api.setPaymentHandles(read.handles)
      setSaved(stored)
      setInputs(handleInputs(stored))
      setDone('saved')
      return true
    } catch (error) {
      setFailure(failureCode(error))
      return false
    } finally {
      setBusy(false)
    }
  }, [api, inputs])

  const remove = useCallback(async () => {
    setBusy(true)
    setFailure(null)
    setDone(null)
    try {
      await api.removePaymentHandles()
      const none = { venmo: null, cash_app: null, paypal: null, zelle: null }
      setSaved(none)
      setInputs(NO_HANDLE_INPUTS)
      setInvalid([])
      setDone('removed')
      return true
    } catch (error) {
      setFailure(failureCode(error))
      return false
    } finally {
      setBusy(false)
    }
  }, [api])

  return { saved, inputs, set, invalid, busy, failure, done, save, remove }
}

/** Whether the account has any payment options saved; `null` until known, `false` if it cannot be known. */
export function useHasPaymentHandles(api: Pick<ExchangeApi, 'paymentHandles'>): boolean | null {
  const [has, setHas] = useState<boolean | null>(null)
  useEffect(() => {
    let cancelled = false
    api.paymentHandles().then(
      (found) => {
        if (!cancelled) setHas(hasAnyHandle(found))
      },
      () => {
        if (!cancelled) setHas(false)
      },
    )
    return () => {
      cancelled = true
    }
  }, [api])
  return has
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
): ShowPaymentOptions {
  const hasHandles = useHasPaymentHandles(api)
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
