import { useInvitationAddress, type BoundAddress } from '@yuppers/shared'
import type { ExchangeView } from '@yuppers/api-client'
import { useEffect, useId, useRef, type FormEvent } from 'react'

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
 * An invitation sent to an address the account does not have: shown
 * masked, with a code sent there to add it (`useInvitationAddress`). If the
 * address is on another of the person's accounts, the offer to combine the
 * two; if the account has another address of that kind, what adding this
 * one does to it, and signing in with the invited address instead.
 */
export function InvitationAddress({ token, sentTo, onOpened, onSignOut }: Props) {
  const { wording, fmt, language } = useI18n()
  const { setAccount } = useSession()
  const w = wording.invitation
  const control = useInvitationAddress(api, token, sentTo, language, onOpened, setAccount)
  const failureId = useId()
  const waitId = useId()
  const codeInput = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (control.step === 'code') codeInput.current?.focus()
  }, [control.step])
  const identifier = sentTo.masked
  const consent = control.codeConsent

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

  return (
    <section className="card invitation-address" aria-labelledby={`${failureId}-heading`}>
      <h2 id={`${failureId}-heading`}>{fmt(w.sentTo, { identifier })}</h2>
      {sentTo.replaces && (
        <p>{sentTo.kind === 'PHONE' ? w.sentToReplacesPhone : w.sentToReplacesEmail}</p>
      )}
      {control.step === 'offer' ? (
        <form noValidate onSubmit={send}>
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
          <Failure code={control.failure} />
          <div className="actions">
            <button
              type="submit"
              className="primary"
              disabled={control.busy || consent.missing}
              aria-describedby={consent.missing ? waitId : undefined}
            >
              {fmt(w.sendAddressCode, { identifier })}
            </button>
          </div>
        </form>
      ) : (
        <form noValidate onSubmit={confirm}>
          <p>{fmt(w.addressCodeSent, { identifier })}</p>
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
              {sentTo.replaces ? w.replaceAndOpen : w.addAndOpen}
            </button>
            <button type="button" disabled={control.busy} onClick={() => void control.sendCode()}>
              {wording.signIn.resend}
            </button>
          </div>
        </form>
      )}
      <p>
        {fmt(w.signInInstead, { identifier })}{' '}
        <button type="button" className="link" onClick={onSignOut}>
          {wording.nav.signOut}
        </button>
      </p>
    </section>
  )
}
