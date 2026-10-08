import type { ExchangeView as Exchange, components } from '@yuppers/api-client'
import {
  fractionDigitsOf,
  fromMinorUnits,
  paymentNote,
  payOptions,
  type PaymentHandles,
  type PayOption,
} from '@yuppers/shared'
import { useEffect, useId, useRef, useState } from 'react'

import { useI18n } from '../app/context'
import { api } from '../lib/api'

type Contribution = components['schemas']['ContributionDto']

interface Props {
  exchange: Exchange
  contribution: Contribution
  otherName: string
  /** Closes the sheet; given the exchange as it was found, if it was read again. */
  onClose(found: Exchange | null): void
  /** "I've paid": opens the existing claim, which the payer still sends themselves. */
  onPaid(found: Exchange | null): void
}

/**
 * "Pay {name} {amount}" (`payments.ts`): the payee's payment options as
 * plain-text buttons that open each app in a new tab, prefilled where the
 * app takes it, and Zelle's address or number to copy. A modal dialog: the
 * page behind it is inert, Escape closes it, and the keyboard returns to
 * the button that opened it.
 *
 * Yuppers moves no money, and nothing is recorded from a link being
 * opened. Once the payer has paid, "I've paid" opens the usual claim, which
 * they send themselves; the payee confirms it as for any payment.
 *
 * The exchange is read again as the sheet opens, so options the payee has
 * stopped showing are never offered.
 */
export function PaySheet({ exchange, contribution, otherName, onClose, onPaid }: Props) {
  const { wording, fmt, money } = useI18n()
  const w = wording.payments
  const dialog = useRef<HTMLDialogElement>(null)
  const heading = useRef<HTMLHeadingElement>(null)
  const id = useId()
  const [found, setFound] = useState<Exchange | null>(null)
  const [handles, setHandles] = useState<PaymentHandles | null | undefined>(
    exchange.payment_options?.theirs,
  )

  const amountMinor = contribution.amount_minor ?? 0
  const amount = money(amountMinor, exchange.currency)
  const plainAmount = fromMinorUnits(amountMinor, fractionDigitsOf(exchange.currency))
  const note = paymentNote(w.note, contribution.description, exchange.display_code)
  const options = payOptions(handles, amountMinor, exchange.currency, note)
  const title = fmt(w.sheetTitle, { name: otherName, amount })

  useEffect(() => {
    const element = dialog.current
    if (!element) return
    // A browser without modal dialogs (and the tests' DOM) shows it in place.
    if (typeof element.showModal === 'function') element.showModal()
    else element.setAttribute('open', '')
    heading.current?.focus()
  }, [])

  useEffect(() => {
    let cancelled = false
    api.getExchange(exchange.id).then(
      (fresh) => {
        if (cancelled) return
        setFound(fresh)
        setHandles(fresh.payment_options?.theirs)
      },
      // Unread, the sheet keeps what the page showed.
      () => {},
    )
    return () => {
      cancelled = true
    }
  }, [exchange.id])

  function shut() {
    const element = dialog.current
    if (typeof element?.close === 'function') element.close()
    else element?.removeAttribute('open')
  }

  function close() {
    shut()
    onClose(found)
  }

  return (
    <dialog
      ref={dialog}
      className="sheet"
      aria-labelledby={`${id}-title`}
      aria-describedby={`${id}-added`}
      onCancel={(event) => {
        event.preventDefault()
        close()
      }}
    >
      <h2 id={`${id}-title`} tabIndex={-1} ref={heading}>
        {title}
      </h2>
      {options.length === 0 ? (
        <p className="notice" id={`${id}-added`}>
          {fmt(w.gone, { name: otherName })}
        </p>
      ) : (
        <>
          <p className="notice notice-warning" id={`${id}-added`}>
            {fmt(w.addedBy, { name: otherName })}
          </p>
          <p className="hint">
            {wording.terms.moneyOutside} {w.appTerms}
          </p>
          <dl className="pay-facts">
            <div>
              <dt>{w.amountLabel}</dt>
              <dd>
                <span>{amount}</span>
                <Copy text={plainAmount} what={w.amountLabel} />
              </dd>
            </div>
            <div>
              <dt>{w.noteLabel}</dt>
              <dd>
                <span dir="auto">{note}</span>
                <Copy text={note} what={w.noteLabel} />
              </dd>
            </div>
          </dl>
          <ul className="pay-options">
            {options.map((option) => (
              <li key={option.app}>
                <Option option={option} amount={amount} />
              </li>
            ))}
          </ul>
        </>
      )}
      <h3>{w.afterHeading}</h3>
      <p>{fmt(w.afterText, { name: otherName })}</p>
      <div className="actions">
        <button
          type="button"
          className="primary"
          onClick={() => {
            shut()
            onPaid(found)
          }}
        >
          {wording.exchange.moneyMoves.CLAIM}
        </button>
        <button type="button" onClick={close}>
          {w.close}
        </button>
      </div>
    </dialog>
  )
}

function Option({ option, amount }: { option: PayOption; amount: string }) {
  const { wording, fmt } = useI18n()
  const w = wording.payments
  const id = useId()

  if (option.app === 'zelle') {
    return (
      <>
        <h3>{w.zelleHeading}</h3>
        <p>{w.zelleNoLinks}</p>
        <p className="pay-handle">
          <span dir="ltr">{option.shown}</span>
          <Copy text={option.shown} what={w.zelleHeading} />
        </p>
        <p className="hint">{fmt(w.enterAmount, { amount })}</p>
      </>
    )
  }

  const told = !option.amountFilled
    ? fmt(w.enterAmount, { amount })
    : option.noteFilled
      ? fmt(w.prefillsAmountAndNote, { amount })
      : fmt(w.prefillsAmount, { amount })
  return (
    <>
      <a
        className="button pay-app"
        href={option.url}
        target="_blank"
        rel="noopener noreferrer"
        aria-describedby={`${id}-handle ${id}-told`}
      >
        {w.open[option.app]}
        <span className="visually-hidden"> {wording.help.newTab}</span>
      </a>
      <p className="pay-handle" id={`${id}-handle`} dir="ltr">
        {fmt(w.handle[option.app], { handle: option.handle })}
      </p>
      <p className="hint" id={`${id}-told`}>
        {told}
      </p>
    </>
  )
}

/** A button that copies one thing, and says when it has. */
function Copy({ text, what }: { text: string; what: string }) {
  const { wording, fmt } = useI18n()
  const w = wording.payments
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')

  async function copy() {
    try {
      await navigator.clipboard.writeText(text)
      setState('copied')
    } catch {
      setState('failed')
    }
  }

  return (
    <>
      <button type="button" className="copy" aria-label={fmt(w.copyWhat, { what })} onClick={() => void copy()}>
        {w.copy}
      </button>
      <span role="status" className={state === 'failed' ? 'field-error' : 'hint'}>
        {state === 'copied' ? w.copied : state === 'failed' ? w.copyFailed : ''}
      </span>
    </>
  )
}
