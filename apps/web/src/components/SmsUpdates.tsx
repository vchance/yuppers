import type { ExchangeView as Exchange } from '@yuppers/api-client'
import { maskPhone, PHONE_EXAMPLE, phoneAsTyped, useSmsUpdates } from '@yuppers/shared'
import { useEffect, useId, useRef, useState, type FormEvent } from 'react'

import { useI18n, useSession } from '../app/context'
import { api } from '../lib/api'
import { useAnnouncement } from '../lib/announce'
import { ConsentCheckbox } from './ConsentCheckbox'
import { Failure, Field, Notice } from './ui'

interface Props {
  exchange: Exchange
}

/** The page on how people opt in to texts, in the language on screen. */
function optInPage(language: string): string {
  return language === 'en' ? '/sms-opt-in' : `/${language}/sms-opt-in`
}

/**
 * "Text updates" on an agreement (DESIGN.md §12, "Yuppers.app agreement
 * updates"): add a US number, checked with a code by text once the box
 * beside it is ticked (`smsCode.verifyNumber`), then a box beside the
 * consent wording, word for word as the terms quote it, and Save. Shown
 * only where the service texts updates, on an agreement sent and not closed
 * (`useSmsUpdates` decides).
 */
export function SmsUpdates({ exchange }: Props) {
  const { wording, fmt, language } = useI18n()
  const session = useSession()
  const w = wording.smsUpdates
  // A number added here is the account's from now on, on every screen.
  const control = useSmsUpdates(
    api,
    exchange,
    language,
    session.setAccount,
    session.account?.email ?? null,
  )
  const id = useId()
  const [phone, setPhone] = useState('')
  const [code, setCode] = useState('')
  const [proofCode, setProofCode] = useState('')
  const codeInput = useRef<HTMLInputElement>(null)
  const phoneInput = useRef<HTMLInputElement>(null)
  const { step, standing, pending, saved, phoneAdded } = control

  // The keyboard follows the step: to the code once it is sent, and back
  // to the number when it is to be changed.
  const previous = useRef(step)
  useEffect(() => {
    if (previous.current === step) return
    if (step === 'enterCode') codeInput.current?.focus()
    if (previous.current === 'enterCode' && step === 'addPhone') phoneInput.current?.focus()
    previous.current = step
  }, [step])

  const number = standing?.phone ? maskPhone(standing.phone) : ''
  const said = saved === 'on' ? fmt(w.on, { phone: number }) : saved === 'off' ? w.off : null
  useAnnouncement(said)

  if (step === 'hidden' || step === 'loading') return null

  const failureId = `${id}-failure`
  const submitPhone = (event: FormEvent) => {
    event.preventDefault()
    void control.requestCode(phone)
  }
  const waitId = `${id}-wait`
  const submitCode = (event: FormEvent) => {
    event.preventDefault()
    void control.verify(code, proofCode)
  }
  const submitConsent = (event: FormEvent) => {
    event.preventDefault()
    void control.save()
  }

  return (
    <section className="card sms-updates" aria-labelledby={`${id}-heading`}>
      <h2 id={`${id}-heading`}>{w.heading}</h2>
      <p>{w.intro}</p>

      {step === 'addPhone' && (
        <form noValidate onSubmit={submitPhone}>
          <p>{w.addPhoneIntro}</p>
          <Field
            label={w.phoneLabel}
            hint={w.phoneHint}
            required
            error={control.invalidPhone ? w.phoneInvalid : undefined}
            problem={control.failure ? failureId : null}
          >
            {(props) => (
              <input
                {...props}
                ref={phoneInput}
                type="tel"
                inputMode="tel"
                autoComplete="tel-national"
                placeholder={PHONE_EXAMPLE}
                dir="ltr"
                value={phone}
                onChange={(event) => {
                  setPhone(event.target.value)
                  // What was ticked was for the number as it was.
                  control.setCodeConsent(false)
                }}
                // Written the American way on leaving: the same number, so
                // the box stays as it was.
                onBlur={() => setPhone(phoneAsTyped)}
              />
            )}
          </Field>
          <Failure code={control.failure} id={failureId} />
          <ConsentCheckbox
            wording={wording.smsCode.verifyNumber}
            checked={control.codeConsent}
            onChange={control.setCodeConsent}
          />
          {!control.codeConsent && (
            <p className="hint" id={waitId}>
              {wording.smsCode.tickToSend}
            </p>
          )}
          <div className="actions">
            <button
              type="submit"
              className="primary"
              disabled={control.busy || !control.codeConsent}
              aria-describedby={control.codeConsent ? undefined : waitId}
            >
              {w.sendCode}
            </button>
          </div>
        </form>
      )}

      {step === 'enterCode' && pending && (
        <form noValidate onSubmit={submitCode}>
          <p>{fmt(w.codeSent, { phone: maskPhone(pending) })}</p>
          <Field
            label={w.codeLabel}
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
                value={code}
                onChange={(event) => setCode(event.target.value)}
              />
            )}
          </Field>
          {control.proofTo && (
            <>
              <p>{fmt(w.proofCodeSent, { email: control.proofTo })}</p>
              <Field
                label={w.proofCodeLabel}
                hint={wording.signIn.codeHint}
                required
                problem={control.failure ? failureId : null}
              >
                {(props) => (
                  <input
                    {...props}
                    type="text"
                    inputMode="numeric"
                    autoComplete="off"
                    maxLength={6}
                    className="code"
                    value={proofCode}
                    onChange={(event) => setProofCode(event.target.value)}
                  />
                )}
              </Field>
            </>
          )}
          <Failure code={control.failure} id={failureId} />
          <div className="actions">
            <button type="submit" className="primary" disabled={control.busy}>
              {w.addPhone}
            </button>
            <button
              type="button"
              className="link"
              disabled={control.busy}
              onClick={() => {
                setCode('')
                setProofCode('')
                control.changePhone()
              }}
            >
              {w.changePhone}
            </button>
          </div>
        </form>
      )}

      {step === 'consent' && (
        <form noValidate onSubmit={submitConsent}>
          {phoneAdded && <Notice>{fmt(w.phoneAdded, { phone: maskPhone(phoneAdded) })}</Notice>}
          <ConsentCheckbox
            wording={w.consent}
            checked={control.checked}
            onChange={control.setChecked}
          />
          <Failure code={control.failure} />
          <div className="actions">
            <button type="submit" className="primary" disabled={control.busy}>
              {w.save}
            </button>
          </div>
          {said ? (
            <p className="notice">{said}</p>
          ) : (
            standing?.on && <p className="hint">{fmt(w.on, { phone: number })}</p>
          )}
        </form>
      )}

      {step === 'optedOut' && <p className="notice">{fmt(w.optedOut, { phone: number })}</p>}

      <p className="learn-more">
        <a href={optInPage(language)} target="_blank" rel="noopener">
          {w.howItWorks}
          <span className="visually-hidden"> {wording.help.newTab}</span>
        </a>
      </p>
    </section>
  )
}
