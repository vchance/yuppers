import {
  PHONE_EXAMPLE,
  phoneAsTyped,
  shownIdentifier,
  useInvitationAddress,
  type BoundAddress,
} from '@yuppers/shared'
import type { ExchangeView } from '@yuppers/api-client'
import { useEffect, useId, useRef, type FormEvent, type ReactNode } from 'react'

import { useI18n, useSession } from '../app/context'
import { api } from '../lib/api'
import { CombineOffer } from './CombineOffer'
import { ConsentCheckbox } from './ConsentCheckbox'
import { Failure, Field } from './ui'

interface Props {
  token: string
  sentTo: BoundAddress
  onOpened(exchange: ExchangeView): void
  onSignOut(): void
}

/**
 * An invitation sent to an address the account does not have: only its
 * kind is said, and the person types the address, which gets a code if it
 * is the one (`useInvitationAddress`). If the address is on another of the
 * person's accounts, the offer to combine the two; if the account has
 * another address of that kind, what adding this one does to it, a code to
 * one of its own first, and signing in with the invited address instead.
 */
export function InvitationAddress({ token, sentTo, onOpened, onSignOut }: Props) {
  const { wording, fmt, language } = useI18n()
  const { account, setAccount } = useSession()
  const w = wording.invitation
  const own = { email: account?.email ?? null, phone: account?.phone ?? null }
  const control = useInvitationAddress(api, token, sentTo, own, language, onOpened, setAccount)
  const failureId = useId()
  const waitId = useId()
  const codeInput = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (control.step === 'code' || control.step === 'proveCode') codeInput.current?.focus()
  }, [control.step])
  const consent = control.codeConsent
  const phone = sentTo.kind === 'PHONE'

  if (control.offer) {
    return (
      <CombineOffer
        offer={control.offer}
        busy={control.busy}
        failure={control.failure}
        onCombine={() => void control.combine()}
        onCancel={control.dismissOffer}
      />
    )
  }

  const send = (event: FormEvent) => {
    event.preventDefault()
    void control.sendCode()
  }
  const confirm = (event: FormEvent) => {
    event.preventDefault()
    void control.confirm()
  }

  const consentBox = (
    <>
      {consent.shown && (
        <ConsentCheckbox
          wording={wording.smsCode.verifyNumber}
          checked={consent.checked}
          onChange={consent.setChecked}
        />
      )}
      {consent.missing && (
        <p className="hint" id={waitId}>
          {wording.smsCode.tickToSend}
        </p>
      )}
    </>
  )

  const codeForm = (sentToText: string, button: string, extra?: ReactNode) => (
    <form noValidate onSubmit={confirm}>
      <p>{fmt(w.addressCodeSent, { identifier: sentToText })}</p>
      <Field
        label={wording.signIn.codeLabel}
        hint={wording.signIn.codeHint}
        required
        problem={control.failure ? failureId : null}
      >
        {(props) => (
          <input
            {...props}
            ref={codeInput}
            type="text"
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={6}
            className="code"
            value={control.code}
            onChange={(event) => control.setCode(event.target.value)}
          />
        )}
      </Field>
      <Failure code={control.failure} id={failureId} />
      <div className="actions">
        <button type="submit" className="primary" disabled={control.busy}>
          {button}
        </button>
        <button type="button" disabled={control.busy} onClick={() => void control.sendCode()}>
          {wording.signIn.resend}
        </button>
        {extra}
      </div>
    </form>
  )

  let body: ReactNode
  if (control.step === 'prove' && control.proveTo) {
    const to = shownIdentifier(control.proveTo)
    const alternative = control.proveAlternative
    body = (
      <form noValidate onSubmit={send}>
        <p>{fmt(wording.identifiers.proveIntro, { identifier: to })}</p>
        {consentBox}
        <Failure code={control.failure} />
        <div className="actions">
          <button
            type="submit"
            className="primary"
            disabled={control.busy || consent.missing}
            aria-describedby={consent.missing ? waitId : undefined}
          >
            {fmt(wording.identifiers.proveSend, { identifier: to })}
          </button>
          {alternative && (
            <button type="button" disabled={control.busy} onClick={control.proveElsewhere}>
              {fmt(wording.identifiers.proveOther, { identifier: shownIdentifier(alternative) })}
            </button>
          )}
        </div>
      </form>
    )
  } else if (control.step === 'proveCode' && control.proveTo) {
    body = codeForm(shownIdentifier(control.proveTo), wording.identifiers.proveConfirm)
  } else if (control.step === 'code' && control.typed) {
    body = codeForm(
      shownIdentifier(control.typed),
      sentTo.replaces ? w.replaceAndOpen : w.addAndOpen,
    )
  } else {
    body = (
      <form noValidate onSubmit={send}>
        <Field
          label={phone ? wording.identifiers.newPhoneLabel : wording.identifiers.newEmailLabel}
          hint={phone ? wording.smsUpdates.phoneHint : undefined}
          required
          error={control.invalidPhone ? wording.smsUpdates.phoneInvalid : undefined}
          problem={control.failure ? failureId : null}
        >
          {(props) =>
            phone ? (
              <input
                {...props}
                type="tel"
                inputMode="tel"
                autoComplete="tel-national"
                placeholder={PHONE_EXAMPLE}
                dir="ltr"
                value={control.input}
                onChange={(event) => control.setInput(event.target.value)}
                onBlur={() => control.setInput(phoneAsTyped(control.input))}
              />
            ) : (
              <input
                {...props}
                type="email"
                autoComplete="email"
                value={control.input}
                onChange={(event) => control.setInput(event.target.value)}
              />
            )
          }
        </Field>
        {consentBox}
        <Failure code={control.failure} id={failureId} />
        <div className="actions">
          <button
            type="submit"
            className="primary"
            disabled={control.busy || consent.missing}
            aria-describedby={consent.missing ? waitId : undefined}
          >
            {wording.identifiers.sendCode}
          </button>
        </div>
      </form>
    )
  }

  return (
    <section className="card invitation-address" aria-labelledby={`${failureId}-heading`}>
      <h2 id={`${failureId}-heading`}>{phone ? w.sentToPhone : w.sentToEmail}</h2>
      {sentTo.replaces && <p>{phone ? w.sentToReplacesPhone : w.sentToReplacesEmail}</p>}
      {body}
      <p>
        {w.signInInstead}{' '}
        <button type="button" className="link" onClick={onSignOut}>
          {wording.nav.signOut}
        </button>
      </p>
    </section>
  )
}
