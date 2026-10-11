import type { Account, ErrorCode } from '@yuppers/api-client'
import { languages, pickLanguage, type Language } from '@yuppers/shared'
import { useState, type FormEvent } from 'react'

import { useI18n, useSession } from '../app/context'
import { rememberLanguage } from '../app/wording'
import { Failure, Field, Notice } from '../components/ui'
import { api, failureCode } from '../lib/api'

/**
 * A name, a confirmation of being 18 or over, and a language. The first two
 * are needed before signing anything (DESIGN.md §14.1), so a new account is
 * asked for them straight after signing in; later the same form edits them.
 * The confirmation of age cannot be taken back, so once given it is stated,
 * not asked again.
 */
export function ProfileForm({ account, first }: { account: Account; first: boolean }) {
  const { wording, language: shown } = useI18n()
  const { setAccount } = useSession()
  const w = wording.profile

  const [name, setName] = useState(account.display_name)
  const [language, setLanguage] = useState<Language>(
    first ? shown : pickLanguage([account.language]),
  )
  const [adult, setAdult] = useState(account.adult_confirmed)
  const [detail, setDetail] = useState(account.notification_detail)
  const [checked, setChecked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [saved, setSaved] = useState(false)

  const nameMissing = name.trim() === ''
  const adultMissing = !account.adult_confirmed && !adult

  async function submit(event: FormEvent) {
    event.preventDefault()
    setChecked(true)
    setSaved(false)
    if (nameMissing || adultMissing) {
      // The keyboard goes to the first thing to fix, once it has been marked;
      // its error is read with it.
      const first = nameMissing ? 'profile-name' : 'profile-adult'
      window.setTimeout(() => document.getElementById(first)?.focus())
      return
    }
    setBusy(true)
    setFailure(null)
    try {
      const updated = await api.updateMe({
        display_name: name.trim(),
        language,
        adult_confirmed: adult ? true : null,
        notification_detail: detail,
      })
      rememberLanguage(language)
      setAccount(updated)
      setSaved(true)
    } catch (error) {
      setFailure(failureCode(error))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form onSubmit={submit} noValidate>
      <Field
        label={w.nameLabel}
        hint={w.nameHint}
        id="profile-name"
        required
        error={checked && nameMissing ? w.nameRequired : null}
      >
        {(control) => (
          <input
            {...control}
            type="text"
            autoComplete="name"
            maxLength={100}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        )}
      </Field>

      <Field label={w.languageLabel}>
        {(control) => (
          <select
            {...control}
            value={language}
            onChange={(event) => setLanguage(event.target.value as Language)}
          >
            {languages.map((info) => (
              <option key={info.code} value={info.code} lang={info.code}>
                {info.name}
              </option>
            ))}
          </select>
        )}
      </Field>

      {account.adult_confirmed ? (
        <p>{w.adultConfirmed}</p>
      ) : (
        <div className="field">
          <label className="check">
            <input
              type="checkbox"
              id="profile-adult"
              required
              checked={adult}
              aria-invalid={checked && adultMissing ? true : undefined}
              aria-describedby={checked && adultMissing ? 'adult-error' : undefined}
              onChange={(event) => setAdult(event.target.checked)}
            />
            <span>{w.adultLabel}</span>
          </label>
          {checked && adultMissing && (
            <p className="field-error" id="adult-error">
              {w.adultRequired}
            </p>
          )}
        </div>
      )}

      {!first && (
        <div className="field">
          <label className="check">
            <input
              type="checkbox"
              role="switch"
              aria-checked={detail}
              id="profile-detail"
              aria-describedby="profile-detail-hint"
              checked={detail}
              onChange={(event) => setDetail(event.target.checked)}
            />
            <span>{w.detailLabel}</span>
          </label>
          <p className="hint" id="profile-detail-hint">
            {w.detailHint}
          </p>
        </div>
      )}

      <Failure code={failure} />
      <div className="actions">
        <button type="submit" className="primary" disabled={busy}>
          {first ? w.continue : w.save}
        </button>
      </div>
      {saved && !first && <Notice>{w.saved}</Notice>}
    </form>
  )
}
