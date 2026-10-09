import type { Account } from '@yuppers/api-client'
import {
  PHONE_EXAMPLE,
  phoneAsTyped,
  shownIdentifier,
  useIdentifiers,
  type IdentifierSlot,
} from '@yuppers/shared'
import { useEffect, useId, useRef, type FormEvent } from 'react'

import { useI18n, useSession } from '../app/context'
import { useAnnouncement } from '../lib/announce'
import { api } from '../lib/api'
import { CombineOffer } from './CombineOffer'
import { ConsentCheckbox } from './ConsentCheckbox'
import { Panel } from './Panel'
import { Failure, Field, Notice } from './ui'

/**
 * The account's email address and phone number, each with Add or Change
 * and Remove (README, "Combining accounts"). Remove is offered only while
 * the account has the other one, and says why when it does not.
 */
export function Identifiers({ account }: { account: Account }) {
  const { wording, language } = useI18n()
  const { setAccount } = useSession()
  const w = wording.identifiers
  const control = useIdentifiers(api, account, setAccount, wording, language)
  const { edit, offer } = control
  const id = useId()
  useAnnouncement(control.done)

  // Back to the button that opened a form, once it closes.
  const openers = useRef(new Map<string, HTMLButtonElement | null>())
  const last = useRef<string | null>(null)
  useEffect(() => {
    if (edit) {
      last.current = `${edit.slot}-${edit.action === 'remove' ? 'remove' : 'edit'}`
    } else if (last.current && !offer) {
      openers.current.get(last.current)?.focus()
      last.current = null
    }
  }, [edit, offer])

  const rows: { slot: IdentifierSlot; label: string; value: string | null | undefined }[] = [
    { slot: 'email', label: wording.profile.emailLabel, value: account.email },
    { slot: 'phone', label: wording.profile.phoneLabel, value: account.phone },
  ]

  return (
    <section aria-labelledby={`${id}-heading`} className="identifiers">
      <h2 id={`${id}-heading`}>{w.heading}</h2>
      <p className="hint">{w.intro}</p>
      {control.done && <Notice>{control.done}</Notice>}
      <dl>
        {rows.map(({ slot, label, value }) => {
          const removable = control.removable(slot)
          const whyNot = `${id}-${slot}-only`
          return (
            <div key={slot} className="identifier-row">
              <dt>{label}</dt>
              <dd>
                <span dir="ltr">{value ? shownIdentifier(value) : w.none}</span>
                <span className="actions inline">
                  <button
                    type="button"
                    ref={(button) => {
                      openers.current.set(`${slot}-edit`, button)
                    }}
                    disabled={control.busy}
                    onClick={() => control.start(slot, value ? 'change' : 'add')}
                  >
                    {value ? w.change : w.add}
                  </button>
                  {value && (
                    <button
                      type="button"
                      ref={(button) => {
                        openers.current.set(`${slot}-remove`, button)
                      }}
                      disabled={control.busy || !removable}
                      aria-describedby={removable ? undefined : whyNot}
                      onClick={() => control.start(slot, 'remove')}
                    >
                      {w.remove}
                    </button>
                  )}
                </span>
                {value && !removable && (
                  <p className="hint" id={whyNot}>
                    {slot === 'email' ? w.onlyEmail : w.onlyPhone}
                  </p>
                )}
              </dd>
            </div>
          )
        })}
      </dl>
      {edit && <EditForm control={control} account={account} />}
      {offer && (
        <CombineOffer
          offer={offer}
          busy={control.busy}
          failure={control.failure}
          onCombine={() => void control.combine()}
          onCancel={control.dismissOffer}
        />
      )}
    </section>
  )
}

function EditForm({
  control,
  account,
}: {
  control: ReturnType<typeof useIdentifiers>
  account: Account
}) {
  const { wording, fmt } = useI18n()
  const w = wording.identifiers
  const edit = control.edit
  const failureId = useId()
  const waitId = useId()
  const input = useRef<HTMLInputElement>(null)
  const codeInput = useRef<HTMLInputElement>(null)
  const step = edit?.step
  useEffect(() => {
    if (step === 'code') codeInput.current?.focus()
  }, [step])
  if (!edit) return null

  const title =
    edit.action === 'remove'
      ? edit.slot === 'email'
        ? w.removeEmailTitle
        : w.removePhoneTitle
      : edit.action === 'change'
        ? edit.slot === 'email'
          ? w.changeEmailTitle
          : w.changePhoneTitle
        : edit.slot === 'email'
          ? w.addEmailTitle
          : w.addPhoneTitle

  const consent = control.codeConsent
  const send = (event: FormEvent) => {
    event.preventDefault()
    void control.sendCode()
  }
  const confirm = (event: FormEvent) => {
    event.preventDefault()
    void control.confirm()
  }

  const codeForm = (sentTo: string, button: string) => (
    <form noValidate onSubmit={confirm}>
      <p>{fmt(w.codeSent, { identifier: shownIdentifier(sentTo) })}</p>
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
        <button type="button" disabled={control.busy} onClick={control.cancel}>
          {wording.common.cancel}
        </button>
      </div>
    </form>
  )

  if (edit.action === 'remove') {
    const staying = shownIdentifier(edit.staying)
    return (
      <Panel title={title}>
        {edit.step === 'explain' ? (
          <form noValidate onSubmit={send}>
            <p>{fmt(w.removeIntro, { staying })}</p>
            <p>{edit.slot === 'phone' ? w.removePhoneNote : w.removeEmailNote}</p>
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
                {fmt(w.sendRemovalCode, { staying })}
              </button>
              <button type="button" disabled={control.busy} onClick={control.cancel}>
                {wording.common.cancel}
              </button>
            </div>
          </form>
        ) : (
          codeForm(edit.staying, w.removeConfirm)
        )}
      </Panel>
    )
  }

  if (edit.step === 'prove' || edit.step === 'proveCode') {
    const to = shownIdentifier(edit.to)
    const alternative = control.proveAlternative
    return (
      <Panel title={title}>
        {edit.step === 'prove' ? (
          <form noValidate onSubmit={send}>
            <p>{fmt(w.proveIntro, { identifier: to })}</p>
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
                {fmt(w.proveSend, { identifier: to })}
              </button>
              {alternative && (
                <button type="button" disabled={control.busy} onClick={control.proveElsewhere}>
                  {fmt(w.proveOther, { identifier: shownIdentifier(alternative) })}
                </button>
              )}
              <button type="button" disabled={control.busy} onClick={control.cancel}>
                {wording.common.cancel}
              </button>
            </div>
          </form>
        ) : (
          codeForm(edit.to, w.proveConfirm)
        )}
      </Panel>
    )
  }

  const current = edit.slot === 'email' ? account.email : account.phone
  return (
    <Panel title={title}>
      {edit.step === 'enter' ? (
        <form noValidate onSubmit={send}>
          {current && <p>{fmt(w.changeNote, { identifier: shownIdentifier(current) })}</p>}
          {current && edit.slot === 'email' && (
            <p>{fmt(w.changeEmailTold, { identifier: current })}</p>
          )}
          <Field
            label={edit.slot === 'email' ? w.newEmailLabel : w.newPhoneLabel}
            hint={edit.slot === 'phone' ? wording.smsUpdates.phoneHint : undefined}
            required
            error={control.invalidPhone ? wording.smsUpdates.phoneInvalid : undefined}
            problem={control.failure ? failureId : null}
          >
            {(props) =>
              edit.slot === 'email' ? (
                <input
                  {...props}
                  ref={input}
                  type="email"
                  autoComplete="email"
                  value={control.input}
                  onChange={(event) => control.setInput(event.target.value)}
                />
              ) : (
                <input
                  {...props}
                  ref={input}
                  type="tel"
                  inputMode="tel"
                  autoComplete="tel-national"
                  placeholder={PHONE_EXAMPLE}
                  dir="ltr"
                  value={control.input}
                  onChange={(event) => control.setInput(event.target.value)}
                  onBlur={() => control.setInput(phoneAsTyped(control.input))}
                />
              )
            }
          </Field>
          <Failure code={control.failure} id={failureId} />
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
          <div className="actions">
            <button
              type="submit"
              className="primary"
              disabled={control.busy || consent.missing}
              aria-describedby={consent.missing ? waitId : undefined}
            >
              {w.sendCode}
            </button>
            <button type="button" disabled={control.busy} onClick={control.cancel}>
              {wording.common.cancel}
            </button>
          </div>
        </form>
      ) : (
        codeForm(edit.identifier, w.confirm)
      )}
    </Panel>
  )
}
