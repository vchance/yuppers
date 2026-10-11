import type { ErrorCode } from '@yuppers/api-client'
import {
  codeWaitText,
  formatPhone,
  identifierRefused,
  identifierToSend,
  phoneAsTyped,
  phoneOffered,
  phoneRefused,
  readsAsPhone,
  signInText,
  smsCodeConsent,
  useResendReady,
  useSignInChannels,
  useSmsCodeConsentBox,
} from '@yuppers/shared'
import { useEffect, useId, useRef, useState, type FormEvent } from 'react'

import { useI18n, useSession } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { ConsentCheckbox } from '../components/ConsentCheckbox'
import { InAppBrowserNote } from '../components/InAppBrowserNote'
import { LegalLink } from '../components/LegalLink'
import { TermsAssent } from '../components/TermsAssent'
import { ErrorNote, Failure, Field, Notice } from '../components/ui'
import { api, failureCode } from '../lib/api'

/**
 * Signing in: an email address or phone number, then the six-digit code sent
 * to it. The first time, this creates the account. The service answers a
 * request for a code the same way whether or not an account exists, and so
 * does this form. A phone number is asked for only where the service can
 * text it (`GET /v1/meta`); elsewhere one typed anyway is stopped here.
 *
 * As soon as what is typed reads as a phone number, an unticked box appears
 * beside the words the terms quote (`smsCode.signIn`), and "Send code"
 * waits for it, saying why: a code goes by text only once it is ticked
 * (`sms-code-consent.ts`). Never ticked to begin with, and unticked again
 * whenever the number changes.
 *
 * A US number is typed the way it is written there, `(856) 548-8780`,
 * with or without +1, and is sent in E.164 (`phone.ts`); leaving the field
 * writes it that way. A number of a country the service does not text is
 * stopped here, in the service's own words.
 *
 * While the code is on its way, the form says to look in the spam folder
 * too, for an email from the address the service names, and offers another
 * code only half a minute after the last (`useResendReady`).
 *
 * Below the form, one line opens the sample yup without signing in, except
 * on the invitation page, where the person has a real yup to read (`example`).
 *
 * In another app's built-in browser, which may not keep anyone signed in,
 * both steps start with a note saying so, and how to open the page in the
 * person's own browser instead (`InAppBrowserNote`).
 */
export function SignIn({ example = true }: { example?: boolean } = {}) {
  const { wording, fmt, language } = useI18n()
  const { setAccount } = useSession()
  const w = wording.signIn

  const [identifier, setIdentifier] = useState('')
  const [sentTo, setSentTo] = useState<string | null>(null)
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [resent, setResent] = useState(false)
  // When the latest code was sent, for offering another a while after.
  const [sentAt, setSentAt] = useState<number | null>(null)
  const resendReady = useResendReady(sentAt)
  // Said under the field, before or instead of what the service answered.
  const [problem, setProblem] = useState<string | null>(null)
  const channels = useSignInChannels(api)
  const text = signInText(w, channels, fmt)
  const phone = phoneOffered(channels)
  const codeInput = useRef<HTMLInputElement>(null)
  const identifierInput = useRef<HTMLInputElement>(null)
  const changing = useRef(false)
  const failureId = useId()
  const waitId = useId()
  // The box, while what is typed would be texted a code.
  const typed = identifier.trim()
  const texted = readsAsPhone(typed) && identifierRefused(typed, channels) === null
  // The box is for the number, not for how it is written: writing it the
  // American way on leaving the field leaves the box as it was.
  const consent = useSmsCodeConsentBox(texted ? identifierToSend(typed) : null)

  // Each step starts with the focus on the one thing it asks for. The first
  // step does so only when the person came back to it; arriving on the page
  // leaves the focus where the browser put it.
  useEffect(() => {
    if (sentTo) codeInput.current?.focus()
    else if (changing.current) identifierInput.current?.focus()
    changing.current = false
  }, [sentTo])

  async function requestCode(to: string, again: boolean) {
    setFailure(null)
    setProblem(null)
    setResent(false)
    if (identifierRefused(to, channels) === 'emailOnly') {
      setProblem(w.emailOnly)
      return
    }
    const refused = phoneRefused(to, channels)
    if (refused) {
      setFailure(refused)
      return
    }
    const target = identifierToSend(to)
    setBusy(true)
    try {
      await api.requestCode(target, consent.checked ? smsCodeConsent(language) : undefined)
      setSentTo(target)
      setSentAt(Date.now())
      setResent(again)
      // The button pressed goes away until another code may be asked for;
      // the keyboard goes back to the code rather than to the top.
      if (again) codeInput.current?.focus()
    } catch (error) {
      const code = failureCode(error)
      // The service's words for this one mention phone numbers.
      if (code === 'INVALID_IDENTIFIER' && !phone && channels) setProblem(w.invalidEmail)
      else setFailure(code)
    } finally {
      setBusy(false)
    }
  }

  async function signIn(event: FormEvent) {
    event.preventDefault()
    if (!sentTo) return
    setBusy(true)
    setFailure(null)
    setResent(false)
    try {
      // The language on screen becomes a new account's language.
      const session = await api.signIn(sentTo, code.trim(), language)
      setAccount(session.account)
    } catch (error) {
      setFailure(failureCode(error))
      setBusy(false)
    }
  }

  if (!sentTo) {
    return (
      // The two steps are separate forms with separate fields, so a browser
      // offering to fill in the code is not looking at the address field.
      <form
        key="identifier"
        noValidate
        onSubmit={(event) => {
          event.preventDefault()
          if (consent.missing) return
          void requestCode(identifier.trim(), false)
        }}
      >
        <InAppBrowserNote />
        <p>{text.intro}</p>
        <Field
          label={text.label}
          hint={text.hint}
          required
          problem={failure || problem ? failureId : null}
        >
          {(control) => (
            <input
              {...control}
              ref={identifierInput}
              // An email field where only an email address is taken; a text
              // field where a phone number may be typed too, which a browser
              // would otherwise take for a malformed email address.
              type={phone ? 'text' : 'email'}
              inputMode="email"
              autoComplete="username"
              autoCapitalize="none"
              spellCheck={false}
              value={identifier}
              onChange={(event) => {
                setIdentifier(event.target.value)
                setProblem(null)
              }}
              onBlur={() => setIdentifier(phoneAsTyped)}
            />
          )}
        </Field>
        {problem ? (
          <ErrorNote id={failureId}>{problem}</ErrorNote>
        ) : (
          <Failure code={failure} id={failureId} />
        )}
        {consent.shown && (
          <ConsentCheckbox
            wording={wording.smsCode.signIn}
            checked={consent.checked}
            onChange={consent.setChecked}
          />
        )}
        {consent.missing && (
          <p className="hint" id={waitId}>
            {wording.smsCode.tickToSend}
          </p>
        )}
        <div className="actions">
          <button
            type="submit"
            className="primary"
            disabled={busy || consent.missing}
            aria-describedby={consent.missing ? waitId : undefined}
          >
            {w.sendCode}
          </button>
        </div>
        {/* Where a code can go to a phone number, the policy on texts. */}
        {phone && (
          <p className="learn-more">
            <LegalLink document="privacy" section="text-messages" label={wording.privacy.smsLink} />
          </p>
        )}
        <p className="learn-more legal-links">
          <LegalLink document="privacy" />
          <LegalLink document="terms" />
        </p>
        {/* A newcomer can see what a yup is before signing in (DESIGN.md §4.3). */}
        {example && (
          <p className="learn-more">
            <Link to={paths.example(language)}>{wording.sample.signInLine}</Link>
          </p>
        )}
      </form>
    )
  }

  const wait = codeWaitText(w, sentTo, channels, fmt)
  return (
    <form key="code" noValidate onSubmit={signIn}>
      <InAppBrowserNote />
      <p>{fmt(w.codeSent, { identifier: formatPhone(sentTo) })}</p>
      {wait && <p>{wait}</p>}
      <Field label={w.codeLabel} hint={w.codeHint} required problem={failure ? failureId : null}>
        {(control) => (
          <input
            {...control}
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
      <Failure code={failure} id={failureId} />
      {resent && <Notice>{w.resent}</Notice>}
      {!resendReady && <p className="hint">{w.resendSoon}</p>}
      {/* Signing in is the assent to both documents (`terms.ts`). */}
      <TermsAssent />
      <div className="actions">
        <button type="submit" className="primary" disabled={busy}>
          {w.submit}
        </button>
        {resendReady && (
          <button type="button" disabled={busy} onClick={() => void requestCode(sentTo, true)}>
            {w.resend}
          </button>
        )}
        <button
          type="button"
          className="link"
          disabled={busy}
          onClick={() => {
            changing.current = true
            // Back to the number, the box is unticked: it is never remembered.
            consent.setChecked(false)
            setSentTo(null)
            setSentAt(null)
            setCode('')
            setFailure(null)
            setProblem(null)
            setResent(false)
          }}
        >
          {text.changeIdentifier}
        </button>
      </div>
    </form>
  )
}
