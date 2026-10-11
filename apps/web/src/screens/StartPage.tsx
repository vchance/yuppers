import type { ErrorCode, ExchangeSummary } from '@yuppers/api-client'
import {
  applyTemplate,
  beginYup,
  deviceTimeZone,
  draftFromCopy,
  fractionDigitsOf,
  labelText,
  TEMPLATES,
  type StartChoice,
  type Template,
} from '@yuppers/shared'
import { useEffect, useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { Link } from '../app/Link'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { ErrorNote, Failure, PageHeading, WithName, Written } from '../components/ui'
import { api, failureCode } from '../lib/api'

/**
 * The first step of a new yup (DESIGN.md §4.4): a short list of common
 * agreements, then the blank form and copying an earlier yup. Choosing makes
 * the draft and opens it in the composer; nothing else is asked here. Names,
 * who it is for and sending come after, as they always did.
 */
export default function StartPage() {
  const { wording } = useI18n()
  const w = wording.templates
  const { account } = useSession()
  const author = account?.display_name ?? ''

  const [mode, setMode] = useState<'choose' | 'copy'>('choose')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [tooMany, setTooMany] = useState(false)

  /** Makes the draft with the working copy a choice starts, and opens it. */
  async function begin(choice: StartChoice, draft: Parameters<typeof beginYup>[3]) {
    setBusy(true)
    setFailure(null)
    setTooMany(false)
    try {
      // Due dates are read in the timezone of whoever starts the exchange.
      const timezone = deviceTimeZone() ?? 'UTC'
      const exchange = await beginYup(api, timezone, choice, draft)
      navigate(paths.exchange(exchange.id))
    } catch (error) {
      const code = failureCode(error)
      // Here the limit is on exchanges started today, not on codes.
      if (code === 'TOO_MANY_REQUESTS') setTooMany(true)
      else setFailure(code)
      setBusy(false)
    }
  }

  function startFrom(template: Template) {
    const words = wording.templates.entries[template.id]
    void begin(
      { kind: 'template', template },
      applyTemplate(template, words, author, () => crypto.randomUUID()),
    )
  }

  return (
    <>
      <PageHeading>{mode === 'copy' ? w.copyHeading : w.chooserTitle}</PageHeading>
      <Failure code={failure} />
      {tooMany && <ErrorNote>{wording.home.tooManyToday}</ErrorNote>}

      {mode === 'choose' ? (
        <>
          <section aria-labelledby="start-common">
            <h2 id="start-common">{w.chooserHeading}</h2>
            <p>{w.chooserIntro}</p>
            <ul className="plain cards choices">
              {TEMPLATES.map((template) => {
                const entry = w.entries[template.id]
                return (
                  <li key={template.id} className="card">
                    <h3>
                      <button
                        type="button"
                        className="choice"
                        disabled={busy}
                        aria-describedby={`${template.id}-summary`}
                        onClick={() => startFrom(template)}
                      >
                        {entry.name}
                      </button>
                    </h3>
                    <p id={`${template.id}-summary`} className="hint">
                      {entry.summary}
                    </p>
                    {entry.warning && <p className="notice notice-warning">{entry.warning}</p>}
                  </li>
                )
              })}
            </ul>
          </section>

          <section aria-labelledby="start-other">
            <h2 id="start-other">{w.orHeading}</h2>
            <ul className="plain cards choices">
              <li className="card">
                <h3>
                  <button
                    type="button"
                    className="choice"
                    disabled={busy}
                    aria-describedby="blank-summary"
                    onClick={() => void begin({ kind: 'blank' }, null)}
                  >
                    {w.blank.name}
                  </button>
                </h3>
                <p id="blank-summary" className="hint">
                  {w.blank.summary}
                </p>
              </li>
              <li className="card">
                <h3>
                  <button
                    type="button"
                    className="choice"
                    disabled={busy}
                    aria-describedby="copy-summary"
                    onClick={() => setMode('copy')}
                  >
                    {w.copy.name}
                  </button>
                </h3>
                <p id="copy-summary" className="hint">
                  {w.copy.summary}
                </p>
              </li>
            </ul>
          </section>

          <section aria-labelledby="start-not-for">
            <h2 id="start-not-for">{w.notForHeading}</h2>
            <p>{w.notFor}</p>
            <p>{w.notForAdvice}</p>
          </section>

          <p>
            <Link to={paths.home}>{wording.nav.exchanges}</Link>
          </p>
        </>
      ) : (
        <CopyPrevious
          busy={busy}
          onBack={() => setMode('choose')}
          onCopy={(draft) => void begin({ kind: 'copy' }, draft)}
          onFailure={setFailure}
        />
      )}
    </>
  )
}

type CopyDraft = Parameters<typeof beginYup>[3]

/**
 * Copying an earlier yup: the person's yups, newest first, closed ones
 * included, then who the copy is for. The other party is asked about every
 * time, and the answer starts as "someone else", so that a name never
 * reaches the wrong person by default (DESIGN.md §4.4).
 */
function CopyPrevious({
  busy,
  onBack,
  onCopy,
  onFailure,
}: {
  busy: boolean
  onBack(): void
  onCopy(draft: CopyDraft): void
  onFailure(code: ErrorCode | null): void
}) {
  const { wording, fmt, moment } = useI18n()
  const w = wording.templates
  const [yups, setYups] = useState<ExchangeSummary[] | null>(null)
  const [chosen, setChosen] = useState<ExchangeSummary | null>(null)
  const [samePerson, setSamePerson] = useState(false)
  const [reading, setReading] = useState(false)
  const [noTerms, setNoTerms] = useState(false)

  useEffect(() => {
    let cancelled = false
    api.listExchanges().then(
      (found) => {
        // A draft that was never sent has no terms to copy.
        if (!cancelled) setYups(found.filter((yup) => yup.state !== 'DRAFT'))
      },
      (error: unknown) => {
        if (!cancelled) onFailure(failureCode(error))
      },
    )
    return () => {
      cancelled = true
    }
  }, [onFailure])

  async function copy(yup: ExchangeSummary) {
    setReading(true)
    setNoTerms(false)
    onFailure(null)
    try {
      const exchange = await api.getExchange(yup.id)
      // The last version: in force, or else the last one sent.
      const last = exchange.in_force_revision ?? exchange.open_revision ?? null
      if (!last) {
        setNoTerms(true)
        setReading(false)
        return
      }
      onCopy(
        draftFromCopy(
          last.terms,
          exchange.you,
          samePerson,
          fractionDigitsOf(exchange.currency),
          () => crypto.randomUUID(),
        ),
      )
    } catch (error) {
      onFailure(failureCode(error))
      setReading(false)
    }
  }

  if (chosen) {
    const otherName = chosen.other_party_name
    return (
      <>
        <p>{w.copyIntro}</p>
        <fieldset>
          <legend>{w.copyForLegend}</legend>
          <label className="check">
            <input
              type="radio"
              name="copy-for"
              checked={!samePerson}
              onChange={() => setSamePerson(false)}
            />
            <span>{w.copyForSomeoneElse}</span>
          </label>
          <label className="check">
            <input
              type="radio"
              name="copy-for"
              checked={samePerson}
              onChange={() => setSamePerson(true)}
            />
            <span>
              {otherName ? (
                <WithName message={w.copyForSame} name={otherName} />
              ) : (
                w.copyForSameNoName
              )}
            </span>
          </label>
        </fieldset>
        {noTerms && <ErrorNote>{w.copyNoTerms}</ErrorNote>}
        <div className="actions">
          <button
            type="button"
            className="primary"
            disabled={busy || reading}
            onClick={() => void copy(chosen)}
          >
            {busy || reading ? w.starting : w.copyStart}
          </button>
          <button type="button" disabled={busy || reading} onClick={() => setChosen(null)}>
            {w.back}
          </button>
        </div>
      </>
    )
  }

  return (
    <>
      <p>{w.copyIntro}</p>
      {!yups && <p>{w.copyLoading}</p>}
      {yups?.length === 0 && <p>{w.copyNone}</p>}
      {yups && yups.length > 0 && (
        <ul className="plain cards">
          {yups.map((yup) => (
            <li key={yup.id} className="card">
              <p>
                {yup.other_party_name ? (
                  <Written inline>{fmt(wording.home.withParty, { name: yup.other_party_name })}</Written>
                ) : (
                  wording.home.noParty
                )}
              </p>
              <p className="tags">
                <span className="tag">
                  {yup.closed_outcome
                    ? wording.outcomes[yup.closed_outcome]
                    : wording.states[yup.state]}
                </span>
              </p>
              <p className="hint">
                {fmt(wording.home.reference, { code: yup.display_code })}
                <br />
                {fmt(wording.home.updated, { date: moment(yup.updated_at) })}
              </p>
              <div className="actions">
                <button
                  type="button"
                  disabled={busy}
                  aria-label={
                    yup.other_party_name
                      ? fmt(w.copyStartNamed, { name: labelText(yup.other_party_name) })
                      : undefined
                  }
                  onClick={() => {
                    setSamePerson(false)
                    setChosen(yup)
                  }}
                >
                  {w.copyStart}
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      <div className="actions">
        <button type="button" onClick={onBack}>
          {w.back}
        </button>
      </div>
    </>
  )
}
