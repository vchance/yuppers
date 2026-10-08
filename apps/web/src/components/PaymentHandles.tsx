import {
  PAYMENT_APPS,
  phoneAsTyped,
  readHandles,
  usePaymentHandles,
  type PaymentApp,
} from '@yuppers/shared'
import { useEffect, useId, useRef, type FormEvent } from 'react'

import { useI18n } from '../app/context'
import { api } from '../lib/api'
import { Failure, Field } from './ui'

/** The wording keys of each app's field, in `payments`. */
const FIELD = {
  venmo: { label: 'venmoLabel', hint: 'venmoHint', invalid: 'venmoInvalid' },
  cash_app: { label: 'cashAppLabel', hint: 'cashAppHint', invalid: 'cashAppInvalid' },
  paypal: { label: 'paypalLabel', hint: 'paypalHint', invalid: 'paypalInvalid' },
  zelle: { label: 'zelleLabel', hint: 'zelleHint', invalid: 'zelleInvalid' },
} as const

/** What each field is, for the browser: none of them is a sign-in, and only Zelle may be an address. */
const INPUT: Record<PaymentApp, { type: 'text' | 'email'; inputMode?: 'email' }> = {
  venmo: { type: 'text' },
  cash_app: { type: 'text' },
  paypal: { type: 'text' },
  zelle: { type: 'text', inputMode: 'email' },
}

/**
 * Payment options on the account screen (`payments.ts`): optional names in
 * Venmo, Cash App, PayPal and Zelle, saved encrypted, shown nowhere until
 * the person turns them on for a yup. Each can be cleared; all can be
 * removed at once.
 */
export function PaymentHandles() {
  const { wording } = useI18n()
  const w = wording.payments
  const form = usePaymentHandles(api)
  const id = useId()
  const said = useRef<HTMLParagraphElement>(null)

  useEffect(() => {
    if (form.done) said.current?.focus()
  }, [form.done])

  function submit(event: FormEvent) {
    event.preventDefault()
    // The keyboard goes to the first entry that is not one.
    const first = readHandles(form.inputs).invalid[0]
    if (first) window.setTimeout(() => document.getElementById(`${id}-${first}`)?.focus())
    void form.save()
  }

  // A US number for Zelle is written the American way on leaving the field;
  // anything else is left as typed, and nothing changes if it already is.
  function showZelleNumber() {
    const shown = phoneAsTyped(form.inputs.zelle)
    if (shown !== form.inputs.zelle) form.set('zelle', shown)
  }

  const anySaved = PAYMENT_APPS.some((app) => Boolean(form.saved?.[app]))

  return (
    <section aria-labelledby={`${id}-heading`} className="payment-handles">
      <h2 id={`${id}-heading`}>{w.heading}</h2>
      <p>{w.intro}</p>
      {form.done && (
        <p className="notice" tabIndex={-1} ref={said} role="status">
          {form.done === 'saved' ? w.saved : w.removed}
        </p>
      )}
      <form noValidate onSubmit={submit}>
        {PAYMENT_APPS.map((app) => (
          <Field
            key={app}
            id={`${id}-${app}`}
            label={w[FIELD[app].label]}
            hint={w[FIELD[app].hint]}
            error={form.invalid.includes(app) ? w[FIELD[app].invalid] : null}
          >
            {(control) => (
              <input
                {...control}
                type={INPUT[app].type}
                inputMode={INPUT[app].inputMode}
                autoComplete="off"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                dir="ltr"
                value={form.inputs[app]}
                disabled={form.saved === null && form.failure === null}
                onChange={(event) => form.set(app, event.target.value)}
                onBlur={app === 'zelle' ? showZelleNumber : undefined}
              />
            )}
          </Field>
        ))}
        <Failure code={form.failure} />
        <div className="actions">
          <button type="submit" className="primary" disabled={form.busy || form.saved === null}>
            {w.save}
          </button>
          {anySaved && (
            <button type="button" disabled={form.busy} onClick={() => void form.remove()}>
              {w.remove}
            </button>
          )}
        </div>
      </form>
    </section>
  )
}
