import {
  addedApps,
  appsToAdd,
  handleShown,
  phoneAsTyped,
  usePaymentOptions,
  type PaymentApp,
  type PaymentOptionsScreen,
} from '@yuppers/shared'
import { useEffect, useId, useRef, type FormEvent } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { Failure, Field, PageHeading, StepHeading } from '../components/ui'
import { api } from '../lib/api'

/** The wording keys of each app's field, in `payments`. */
const FIELD = {
  venmo: { label: 'venmoLabel', hint: 'venmoHint', invalid: 'venmoInvalid' },
  cash_app: { label: 'cashAppLabel', hint: 'cashAppHint', invalid: 'cashAppInvalid' },
  paypal: { label: 'paypalLabel', hint: 'paypalHint', invalid: 'paypalInvalid' },
  zelle: { label: 'zelleLabel', hint: 'zelleHint', invalid: 'zelleInvalid' },
} as const

/**
 * The payment options screen, at `/account/payments` (`payments.ts`):
 * optional names in Venmo, Cash App, PayPal and Zelle, saved encrypted and
 * shown nowhere until the person turns them on for a yup. The ones added
 * are listed, each with Edit and Remove; "Add a payment option" asks which
 * app, from those not added yet, then shows that app's one field. Removing
 * asks first, in a dialog on the page, and says when it is the last one,
 * which stops showing them on every yup.
 *
 * The focus follows the person: to the heading of each step as it opens,
 * to what was said once something is saved or removed, and back to the
 * button they came from when they cancel or keep an option.
 */
export default function PaymentOptionsPage() {
  const { wording, fmt } = useI18n()
  const w = wording.payments
  const screen = usePaymentOptions(api)
  const id = useId()
  const said = useRef<HTMLParagraphElement>(null)
  const addButton = useRef<HTMLButtonElement>(null)
  const rowButtons = useRef(new Map<string, HTMLButtonElement>())
  // The Remove button whose dialog was last opened, to go back to if the option is kept.
  const asked = useRef<PaymentApp | null>(null)

  const { done, confirming, step } = screen
  useEffect(() => {
    if (!done) return
    if (done.what === 'cancelled') {
      const back = done.app ? rowButtons.current.get(`edit-${done.app}`) : addButton.current
      back?.focus()
    } else {
      said.current?.focus()
    }
  }, [done])

  useEffect(() => {
    if (confirming || !asked.current) return
    // Kept: back to its Remove button. Removed: `done` takes the focus instead.
    rowButtons.current.get(`remove-${asked.current}`)?.focus()
    asked.current = null
  }, [confirming])

  const saved = screen.saved
  const added = addedApps(saved)
  const toAdd = appsToAdd(saved)
  const doneText =
    done?.what === 'saved'
      ? fmt(w.savedOne, { app: w.apps[done.app] })
      : done?.what === 'removed'
        ? fmt(done.last ? w.removedLast : w.removedOne, { app: w.apps[done.app] })
        : null

  return (
    <div className="payment-options-page">
      <p className="back">
        <Link to={paths.account}>{w.back}</Link>
      </p>
      <PageHeading>{w.heading}</PageHeading>
      <p>{w.intro}</p>

      {saved === null && screen.loadFailure === null && <p>{wording.common.loading}</p>}
      {screen.loadFailure && (
        <>
          <Failure code={screen.loadFailure} />
          <div className="actions">
            <button type="button" onClick={screen.reload}>
              {wording.common.tryAgain}
            </button>
          </div>
        </>
      )}

      {saved !== null && step.name === 'list' && (
        <>
          {doneText && (
            <p className="notice" tabIndex={-1} ref={said} role="status">
              {doneText}
            </p>
          )}
          <Failure code={screen.failure} />
          {added.length === 0 ? (
            <p className="payment-empty">{w.empty}</p>
          ) : (
            <ul className="plain payment-list">
              {added.map((app) => (
                <li key={app} className="card payment-row">
                  <div className="payment-row-text">
                    <h2>{w.apps[app]}</h2>
                    <p dir="ltr">{handleShown(app, saved)}</p>
                  </div>
                  <div className="actions">
                    <button
                      type="button"
                      aria-label={fmt(w.editWhat, { app: w.apps[app] })}
                      disabled={screen.busy}
                      ref={(element) => {
                        if (element) rowButtons.current.set(`edit-${app}`, element)
                        else rowButtons.current.delete(`edit-${app}`)
                      }}
                      onClick={() => screen.edit(app)}
                    >
                      {w.edit}
                    </button>
                    <button
                      type="button"
                      aria-label={fmt(w.removeWhat, { app: w.apps[app] })}
                      aria-haspopup="dialog"
                      disabled={screen.busy}
                      ref={(element) => {
                        if (element) rowButtons.current.set(`remove-${app}`, element)
                        else rowButtons.current.delete(`remove-${app}`)
                      }}
                      onClick={() => {
                        asked.current = app
                        screen.askToRemove(app)
                      }}
                    >
                      {w.removeOne}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
          {toAdd.length > 0 ? (
            <div className="actions">
              <button
                type="button"
                className="primary"
                ref={addButton}
                disabled={screen.busy}
                onClick={screen.startAdding}
              >
                {w.add}
              </button>
            </div>
          ) : (
            <p className="hint">{w.allAdded}</p>
          )}
        </>
      )}

      {saved !== null && step.name === 'pick' && (
        <section aria-labelledby={`${id}-pick`}>
          <StepHeading id={`${id}-pick`}>{w.pickHeading}</StepHeading>
          <p>{w.pickIntro}</p>
          <ul className="plain payment-picks">
            {toAdd.map((app) => (
              <li key={app}>
                <button type="button" onClick={() => screen.pick(app)}>
                  {w.apps[app]}
                </button>
              </li>
            ))}
          </ul>
          <div className="actions">
            <button type="button" className="link" onClick={screen.cancel}>
              {wording.common.cancel}
            </button>
          </div>
        </section>
      )}

      {saved !== null && step.name === 'field' && (
        <OptionField screen={screen} app={step.app} adding={step.adding} />
      )}

      {confirming && (
        <ConfirmRemove
          app={confirming}
          last={added.length === 1 && added[0] === confirming}
          busy={screen.busy}
          onRemove={() => void screen.remove()}
          onKeep={screen.keep}
        />
      )}
    </div>
  )
}

/** Adding or editing one app's option: its one field, as on the old form. */
function OptionField({
  screen,
  app,
  adding,
}: {
  screen: PaymentOptionsScreen
  app: PaymentApp
  adding: boolean
}) {
  const { wording, fmt } = useI18n()
  const w = wording.payments
  const id = useId()
  const input = useRef<HTMLInputElement>(null)
  const title = fmt(adding ? w.addHeading : w.editHeading, { app: w.apps[app] })

  function submit(event: FormEvent) {
    event.preventDefault()
    void screen.save().then((ok) => {
      // Not one: the keyboard goes back to the field, which says why.
      if (!ok) input.current?.focus()
    })
  }

  // A US number for Zelle is written the American way on leaving the field;
  // anything else is left as typed, and nothing changes if it already is.
  function showZelleNumber() {
    const shown = phoneAsTyped(screen.input)
    if (shown !== screen.input) screen.setInput(shown)
  }

  return (
    <section aria-labelledby={`${id}-heading`}>
      <StepHeading id={`${id}-heading`}>{title}</StepHeading>
      <form noValidate onSubmit={submit}>
        <Field
          id={`${id}-field`}
          label={w[FIELD[app].label]}
          hint={w[FIELD[app].hint]}
          error={screen.invalid ? w[FIELD[app].invalid] : null}
          required
        >
          {(control) => (
            <input
              {...control}
              ref={input}
              type="text"
              inputMode={app === 'zelle' ? 'email' : undefined}
              autoComplete="off"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              dir="ltr"
              value={screen.input}
              onChange={(event) => screen.setInput(event.target.value)}
              onBlur={app === 'zelle' ? showZelleNumber : undefined}
            />
          )}
        </Field>
        <Failure code={screen.failure} />
        <div className="actions">
          <button type="submit" className="primary" disabled={screen.busy}>
            {w.saveOne}
          </button>
          <button type="button" disabled={screen.busy} onClick={screen.cancel}>
            {wording.common.cancel}
          </button>
        </div>
      </form>
    </section>
  )
}

/**
 * "Remove Venmo?": a modal dialog on the page, never the browser's own.
 * The page behind is inert, Escape keeps the option, and the focus starts
 * on keeping it, the choice that loses nothing.
 */
function ConfirmRemove({
  app,
  last,
  busy,
  onRemove,
  onKeep,
}: {
  app: PaymentApp
  last: boolean
  busy: boolean
  onRemove(): void
  onKeep(): void
}) {
  const { wording, fmt } = useI18n()
  const w = wording.payments
  const id = useId()
  const dialog = useRef<HTMLDialogElement>(null)
  const keep = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    const element = dialog.current
    if (!element) return
    // A browser without modal dialogs (and the tests' DOM) shows it in place.
    if (typeof element.showModal === 'function') element.showModal()
    else element.setAttribute('open', '')
    keep.current?.focus()
    return () => {
      if (typeof element.close === 'function' && element.open) element.close()
    }
  }, [])

  return (
    <dialog
      ref={dialog}
      className="sheet confirm"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby={`${id}-title`}
      aria-describedby={`${id}-text`}
      onCancel={(event) => {
        event.preventDefault()
        if (!busy) onKeep()
      }}
    >
      <h2 id={`${id}-title`}>{fmt(w.confirmTitle, { app: w.apps[app] })}</h2>
      <p id={`${id}-text`}>{last ? w.confirmLast : w.confirmText}</p>
      <div className="actions">
        <button type="button" className="primary" disabled={busy} onClick={onRemove}>
          {fmt(w.removeWhat, { app: w.apps[app] })}
        </button>
        <button type="button" ref={keep} disabled={busy} onClick={onKeep}>
          {w.keep}
        </button>
      </div>
    </dialog>
  )
}
