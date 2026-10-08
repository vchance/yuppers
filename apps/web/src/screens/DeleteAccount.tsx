import type { Account } from '@yuppers/api-client'
import { formatPhone, useAccountDeletion, useSignInChannels } from '@yuppers/shared'
import { useEffect, useId, useRef, useState, type FormEvent } from 'react'

import { useI18n, useSession } from '../app/context'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { ConsentCheckbox } from '../components/ConsentCheckbox'
import { HelpLink } from '../components/HelpLink'
import { LegalLink } from '../components/LegalLink'
import { Panel } from '../components/Panel'
import { Failure, Field, Notice, Written } from '../components/ui'
import { api } from '../lib/api'

/**
 * Deleting the account, from the account page (DESIGN.md §4.1). It cannot be
 * undone, so it is said in full first, what goes and what stays, then proved
 * with a one-time code, then asked once more.
 */
export default function DeleteAccount({ account }: { account: Account }) {
  const { wording } = useI18n()
  const w = wording.deletion
  const [open, setOpen] = useState(false)
  const opener = useRef<HTMLButtonElement>(null)
  const closed = useRef(false)

  // Cancelling puts the keyboard back on the button that opened this.
  useEffect(() => {
    if (!open && closed.current) opener.current?.focus()
  }, [open])

  return (
    <section aria-labelledby="deletion-heading">
      <h2 id="deletion-heading">{w.heading}</h2>
      <HelpLink place="deletion" />
      <p className="learn-more">
        <LegalLink
          document="privacy"
          section="deleting-your-account"
          label={wording.privacy.deletion}
        />
      </p>
      {open ? (
        <Steps
          account={account}
          onCancel={() => {
            closed.current = true
            setOpen(false)
          }}
        />
      ) : (
        <div className="actions">
          <button type="button" ref={opener} onClick={() => setOpen(true)}>
            {w.open}
          </button>
        </div>
      )}
    </section>
  )
}

function Steps({ account, onCancel }: { account: Account; onCancel(): void }) {
  const { wording, fmt, language } = useI18n()
  const { setAccount } = useSession()
  const w = wording.deletion
  const id = useId()

  // A phone number is offered for the code only where the service can text it.
  const channels = useSignInChannels(api)
  const deletion = useAccountDeletion(
    api,
    account,
    () => {
      // The service has ended every session and taken back the cookie; this
      // page stops acting as the account and goes back to the way in.
      setAccount(null)
      navigate(paths.home)
    },
    channels,
    language,
  )
  const { step, preview, destination, busy } = deletion
  // A phone number as the screens show one, `(856) 548-8780`.
  const identifier = formatPhone(destination?.identifier ?? '')

  // The code step starts with the keyboard on the one thing it asks for.
  const codeInput = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (step === 'code') codeInput.current?.focus()
  }, [step])
  // Sent without a code, the keyboard goes back to it, and its error is read with it.
  const codeMissing = deletion.codeMissing
  useEffect(() => {
    if (codeMissing) codeInput.current?.focus()
  }, [codeMissing])
  const failureId = useId()
  const waitId = useId()
  const consent = deletion.codeConsent

  if (step === 'confirm') {
    return (
      <Panel key="confirm" title={w.confirmHeading}>
        <p>{w.confirmBody}</p>
        <Failure code={deletion.failure} />
        <div className="actions">
          <button
            type="button"
            className="primary"
            disabled={busy}
            onClick={() => void deletion.confirm()}
          >
            {w.confirm}
          </button>
          <button type="button" disabled={busy} onClick={deletion.back}>
            {w.back}
          </button>
          <button type="button" className="link" disabled={busy} onClick={onCancel}>
            {wording.common.cancel}
          </button>
        </div>
      </Panel>
    )
  }

  if (step === 'code') {
    const submit = (event: FormEvent) => {
      event.preventDefault()
      deletion.review()
    }
    return (
      <form key="code" noValidate onSubmit={submit}>
        <p>{fmt(w.codeSent, { identifier })}</p>
        <Field
          label={wording.signIn.codeLabel}
          hint={wording.signIn.codeHint}
          required
          problem={deletion.failure ? failureId : null}
          error={deletion.codeMissing ? w.codeRequired : null}
        >
          {(control) => (
            <input
              {...control}
              ref={codeInput}
              type="text"
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={6}
              className="code"
              value={deletion.code}
              onChange={(event) => deletion.setCode(event.target.value)}
            />
          )}
        </Field>
        <Failure code={deletion.failure} id={failureId} />
        {deletion.resent && <Notice>{wording.signIn.resent}</Notice>}
        <div className="actions">
          <button type="submit" className="primary" disabled={busy}>
            {w.continue}
          </button>
          <button type="button" disabled={busy} onClick={() => void deletion.sendCode()}>
            {wording.signIn.resend}
          </button>
          <button type="button" className="link" disabled={busy} onClick={onCancel}>
            {wording.common.cancel}
          </button>
        </div>
      </form>
    )
  }

  const open = preview ? preview.open_proposals + preview.agreements_in_force : 0
  return (
    <Panel key="explain" title={w.intro}>
      <h3>{w.deletedHeading}</h3>
      <ul>
        <li>{w.deletedAccount}</li>
        <li>{w.deletedData}</li>
      </ul>
      <p>{w.signUpAgain}</p>

      <h3>{w.exchangesHeading}</h3>
      {!preview && !deletion.previewFailure && <p>{wording.common.loading}</p>}
      {!preview && deletion.previewFailure && (
        <>
          <Failure code={deletion.previewFailure} />
          <p>{w.loadFailed}</p>
          <div className="actions">
            <button type="button" onClick={deletion.loadPreview}>
              {wording.common.tryAgain}
            </button>
          </div>
        </>
      )}
      {preview && preview.drafts + open === 0 && <p>{w.nothingOpen}</p>}
      {preview && preview.drafts + open > 0 && (
        <ul>
          {preview.drafts > 0 && <li>{fmt(w.drafts, { count: preview.drafts })}</li>}
          {preview.open_proposals > 0 && (
            <li>{fmt(w.openProposals, { count: preview.open_proposals })}</li>
          )}
          {preview.agreements_in_force > 0 && (
            <li>{fmt(w.agreementsInForce, { count: preview.agreements_in_force })}</li>
          )}
        </ul>
      )}
      {preview && preview.agreements_in_force > 0 && <p>{w.agreementsStand}</p>}
      {open > 0 && <p>{w.otherPartyTold}</p>}

      <h3>{w.keptHeading}</h3>
      <ul>
        <li>{w.keptAgreements}</li>
        <li>{w.keptReports}</li>
      </ul>

      {deletion.destinations.length > 1 ? (
        <>
          <p>{w.codeChoice}</p>
          <fieldset>
            <legend>{w.sendTo}</legend>
            {deletion.destinations.map((option) => (
              <label className="check" key={option.channel}>
                <input
                  type="radio"
                  name={`${id}-channel`}
                  value={option.channel}
                  checked={option.channel === destination?.channel}
                  onChange={() => deletion.choose(option.channel)}
                />
                <Written inline>{formatPhone(option.identifier)}</Written>
              </label>
            ))}
          </fieldset>
        </>
      ) : (
        <p>{fmt(w.codeIntro, { identifier })}</p>
      )}

      {/* A code by text only once the box beside the number is ticked. */}
      {consent.shown && (
        <ConsentCheckbox
          wording={wording.smsCode.deleteAccount}
          checked={consent.checked}
          onChange={consent.setChecked}
        />
      )}
      {consent.missing && (
        <p className="hint" id={waitId}>
          {wording.smsCode.tickToSend}
        </p>
      )}
      <Failure code={deletion.failure} />
      <div className="actions">
        <button
          type="button"
          className="primary"
          disabled={busy || !preview || !destination || consent.missing}
          aria-describedby={consent.missing ? waitId : undefined}
          onClick={() => void deletion.sendCode()}
        >
          {w.sendCode}
        </button>
        <button type="button" disabled={busy} onClick={onCancel}>
          {wording.common.cancel}
        </button>
      </div>
    </Panel>
  )
}
