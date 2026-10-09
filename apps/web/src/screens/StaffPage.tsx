import type { ErrorCode } from '@yuppers/api-client'
import { documentOf, failureCode, moneyIds, recordMoments, termsOfRevision } from '@yuppers/shared'
import { useCallback, useEffect, useId, useMemo, useState, type FormEvent } from 'react'

import { useI18n, useSession } from '../app/context'
import { Link } from '../app/Link'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { BuildVersion } from '../components/BuildVersion'
import { EventList } from '../components/EventList'
import { Panel } from '../components/Panel'
import { ErrorNote, Failure, Field, Notice, PageHeading, WithName, Written } from '../components/ui'
import { api } from '../lib/api'
import {
  staffApi,
  type HiddenContent,
  type QueuedReport,
  type ReportDetail,
  type ReviewOutcome,
  type Suspension,
} from '../lib/staff-api'
import { Version } from './RecordPage'
import './record.css'

/*
 * Staff review of abuse reports (DESIGN.md §9): the queue, oldest first and
 * each with its age, an open report with the reported exchange's record and
 * the decision, and the suspensions and hidden content that review put in
 * place, each of which can be undone with a note.
 *
 * Nothing in the app links here. To anyone who is not a reviewer the service
 * answers "not found" and so does this page. Its words are in the `staff`
 * part of the wording files like every other screen's; reviewers working in
 * English only is acceptable, and Spanish is there too.
 */

const OUTCOMES: readonly ReviewOutcome[] = [
  'DISMISSED',
  'CONTENT_HIDDEN',
  'ACCOUNT_SUSPENDED',
  'CONTENT_HIDDEN_AND_ACCOUNT_SUSPENDED',
]

/** What the last decision was, said on the queue the reviewer is taken back to. */
let decided: ReviewOutcome | null = null

export default function StaffPage({ report }: { report: string | null }) {
  return report ? <ReportReview id={report} /> : <Queue />
}

/** Loads something once, and again on `reload`. */
function useLoaded<T>(load: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const reload = useCallback(() => {
    load().then(
      (found) => {
        setData(found)
        setFailure(null)
      },
      (error: unknown) => setFailure(failureCode(error)),
    )
  }, [load])
  useEffect(reload, [reload])
  return { data, failure, reload }
}

/**
 * A refusal that stops the whole page: someone who is not a reviewer sees
 * the page that is not there; a reviewer whose sign-in is too old is asked
 * to sign in again, here.
 */
function Stopped({ code }: { code: ErrorCode }) {
  const { wording } = useI18n()
  const { setAccount } = useSession()
  const [leaving, setLeaving] = useState(false)

  if (code === 'NOT_FOUND') {
    return (
      <>
        <PageHeading>{wording.common.notFoundTitle}</PageHeading>
        <p>{wording.common.notFoundBody}</p>
        <p>
          <Link to={paths.home}>{wording.common.goHome}</Link>
        </p>
      </>
    )
  }

  async function signOut() {
    setLeaving(true)
    try {
      await api.signOut()
    } catch {
      // Whether or not the service heard, this browser stops acting as the account.
    }
    // The sign-in form takes this page's place, and the page comes back after it.
    setAccount(null)
  }

  return (
    <>
      <PageHeading>{wording.staff.title}</PageHeading>
      <Failure code={code} />
      {code === 'SESSION_TOO_OLD' && (
        <div className="actions">
          <button type="button" className="primary" disabled={leaving} onClick={signOut}>
            {wording.nav.signOut}
          </button>
        </div>
      )}
    </>
  )
}

function stops(code: ErrorCode | null): code is 'NOT_FOUND' | 'SESSION_TOO_OLD' {
  return code === 'NOT_FOUND' || code === 'SESSION_TOO_OLD'
}

/** How long a report has waited, in hours, or in minutes under the first hour. */
function useWaiting() {
  const { wording, fmt } = useI18n()
  return (seconds: number) => {
    const hours = Math.floor(seconds / 3600)
    return hours >= 1
      ? fmt(wording.staff.waiting, { hours })
      : fmt(wording.staff.waitingMinutes, { minutes: Math.floor(seconds / 60) })
  }
}

// ---- The queue -----------------------------------------------------------------

function Queue() {
  const { wording, fmt } = useI18n()
  const w = wording.staff
  const load = useCallback(() => staffApi.queue(), [])
  const { data: queue, failure } = useLoaded(load)
  const [done] = useState(() => {
    const outcome = decided
    decided = null
    return outcome
  })

  if (stops(failure)) return <Stopped code={failure} />

  return (
    <>
      <PageHeading>{w.title}</PageHeading>
      {done && <Notice>{w.outcomeDone[done]}</Notice>}
      <p>{fmt(w.intro, { hours: queue?.review_within_hours ?? 24 })}</p>

      <section aria-labelledby="staff-queue">
        <h2 id="staff-queue">{w.queueHeading}</h2>
        <Failure code={failure} />
        {!queue && !failure && <p>{wording.common.loading}</p>}
        {queue?.reports.length === 0 && <p>{w.queueEmpty}</p>}
        {queue && queue.reports.length > 0 && (
          <ul className="plain">
            {queue.reports.map((report) => (
              <QueueEntry key={report.id} report={report} />
            ))}
          </ul>
        )}
      </section>

      <Suspensions />
      <Hidden />
      <BuildVersion />
    </>
  )
}

function QueueEntry({ report }: { report: QueuedReport }) {
  const { wording, fmt, moment } = useI18n()
  const w = wording.staff
  const waiting = useWaiting()
  return (
    <li className={report.overdue ? 'card card-overdue' : 'card'}>
      <Link to={paths.staffReport(report.id)} className="card-link">
        {report.display_code ? fmt(w.reportLink, { code: report.display_code }) : w.reportLinkNoYup}
      </Link>
      <p className="tags">
        {report.overdue && <span className="tag tag-alert">{w.overdue}</span>}
        <span className="tag">{wording.safety.reasons[report.reason]}</span>
      </p>
      <p className="hint">
        {waiting(report.age_seconds)}
        <br />
        {fmt(w.filed, { date: moment(report.created_at) })}
      </p>
    </li>
  )
}

/** A note asked for before an action, with a way to back out. */
function NoteAction({
  title,
  text,
  required,
  confirm,
  onConfirm,
  onCancel,
}: {
  title: string
  text: string
  required: boolean
  confirm: string
  onConfirm(note: string): Promise<void>
  onCancel(): void
}) {
  const { wording } = useI18n()
  const w = wording.staff
  const id = useId()
  const [note, setNote] = useState('')
  const [checked, setChecked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const missing = checked && required && note.trim() === ''

  async function submit(event: FormEvent) {
    event.preventDefault()
    setChecked(true)
    if (required && note.trim() === '') {
      document.getElementById(`${id}-note`)?.focus()
      return
    }
    setBusy(true)
    setFailure(null)
    try {
      await onConfirm(note.trim())
    } catch (error) {
      setFailure(failureCode(error))
      setBusy(false)
    }
  }

  return (
    <Panel title={title}>
      <form noValidate onSubmit={submit}>
        <p>{text}</p>
        <Field
          label={required ? w.noteLabel : w.noteOptionalLabel}
          id={`${id}-note`}
          required={required}
          error={missing ? w.noteRequired : null}
        >
          {(control) => (
            <textarea
              {...control}
              rows={3}
              maxLength={1000}
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          )}
        </Field>
        <Failure code={failure} />
        <div className="actions">
          <button type="submit" className="primary" disabled={busy}>
            {confirm}
          </button>
          <button type="button" disabled={busy} onClick={onCancel}>
            {wording.common.cancel}
          </button>
        </div>
      </form>
    </Panel>
  )
}

/**
 * Puts the keyboard back on the button that opened a panel. The panel took
 * that button's place, so it is found again by its id once it is back.
 */
function focusSoon(id: string) {
  window.setTimeout(() => document.getElementById(id)?.focus(), 0)
}

function Suspensions() {
  const { wording, fmt, moment } = useI18n()
  const w = wording.staff
  const load = useCallback(() => staffApi.suspensions(), [])
  const { data: suspended, failure, reload } = useLoaded(load)
  const [open, setOpen] = useState<string | null>(null)
  const [lifted, setLifted] = useState(false)

  if (stops(failure)) return null
  return (
    <section aria-labelledby="staff-suspensions">
      <h2 id="staff-suspensions">{w.suspensionsHeading}</h2>
      <Failure code={failure} />
      {lifted && <Notice>{w.lifted}</Notice>}
      {suspended?.length === 0 && <p>{w.suspensionsEmpty}</p>}
      {suspended && suspended.length > 0 && (
        <ul className="plain">
          {suspended.map((entry: Suspension) => (
            <li key={entry.account_id} className="card">
              {entry.name && (
                <p>
                  <Written inline>{entry.name}</Written>
                </p>
              )}
              <p className="hint">
                {fmt(w.account, { id: entry.account_id })}
                {entry.suspended_at && (
                  <>
                    <br />
                    {fmt(w.suspendedSince, { date: moment(entry.suspended_at) })}
                  </>
                )}
              </p>
              {entry.note && <Written>{entry.note}</Written>}
              {open === entry.account_id ? (
                <NoteAction
                  title={w.lift}
                  text={w.liftText}
                  required
                  confirm={w.lift}
                  onConfirm={async (note) => {
                    await staffApi.lift(entry.account_id, note)
                    setOpen(null)
                    setLifted(true)
                    reload()
                  }}
                  onCancel={() => {
                    setOpen(null)
                    focusSoon(`lift-${entry.account_id}`)
                  }}
                />
              ) : (
                <div className="actions">
                  <button
                    type="button"
                    id={`lift-${entry.account_id}`}
                    onClick={() => {
                      setLifted(false)
                      setOpen(entry.account_id)
                    }}
                  >
                    {w.lift}
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function Hidden() {
  const { wording, fmt, moment } = useI18n()
  const w = wording.staff
  const load = useCallback(() => staffApi.hidden(), [])
  const { data: hidden, failure, reload } = useLoaded(load)
  const [open, setOpen] = useState<string | null>(null)
  const [restored, setRestored] = useState(false)
  const key = (entry: HiddenContent) => `${entry.exchange_id}/${entry.account_id}`

  if (stops(failure)) return null
  return (
    <section aria-labelledby="staff-hidden">
      <h2 id="staff-hidden">{w.hiddenHeading}</h2>
      <Failure code={failure} />
      {restored && <Notice>{w.restored}</Notice>}
      {hidden?.length === 0 && <p>{w.hiddenEmpty}</p>}
      {hidden && hidden.length > 0 && (
        <ul className="plain">
          {hidden.map((entry) => (
            <li key={key(entry)} className="card">
              <p>{fmt(wording.home.reference, { code: entry.display_code })}</p>
              <p className="hint">
                {entry.name && (
                  <>
                    <WithName message={w.nameInYup} name={entry.name} />
                    <br />
                  </>
                )}
                {fmt(w.account, { id: entry.account_id })}
                <br />
                {fmt(w.hiddenSince, { date: moment(entry.hidden_at) })}
              </p>
              {open === key(entry) ? (
                <NoteAction
                  title={w.restore}
                  text={w.restoreText}
                  required
                  confirm={w.restore}
                  onConfirm={async (note) => {
                    await staffApi.restore(entry.exchange_id, entry.account_id, note)
                    setOpen(null)
                    setRestored(true)
                    reload()
                  }}
                  onCancel={() => {
                    setOpen(null)
                    focusSoon(`restore-${key(entry)}`)
                  }}
                />
              ) : (
                <div className="actions">
                  <button
                    type="button"
                    id={`restore-${key(entry)}`}
                    onClick={() => {
                      setRestored(false)
                      setOpen(key(entry))
                    }}
                  >
                    {w.restore}
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

// ---- One report ----------------------------------------------------------------

function ReportReview({ id }: { id: string }) {
  const { wording } = useI18n()
  const load = useCallback(() => staffApi.report(id), [id])
  const { data: detail, failure } = useLoaded(load)

  if (stops(failure)) return <Stopped code={failure} />
  if (!detail) {
    return (
      <>
        <p>
          <Link to={paths.staff}>{wording.staff.back}</Link>
        </p>
        {failure ? <ErrorNote>{wording.errors[failure]}</ErrorNote> : <p>{wording.common.loading}</p>}
      </>
    )
  }
  return <Report detail={detail} />
}

function Report({ detail }: { detail: ReportDetail }) {
  const { wording, fmt, moment } = useI18n()
  const w = wording.staff
  const waiting = useWaiting()
  const { report, reporter, subject, record } = detail
  const [open, setOpen] = useState<ReviewOutcome | null>(null)

  const who = (account: NonNullable<ReportDetail['subject']>) => (
    <>
      {fmt(w.account, { id: account.id })}
      {' · '}
      {w.standing[account.status]}
      {account.merged_into && (
        <>
          <br />
          {fmt(w.mergedInto, {
            id: account.merged_into,
            date: account.merged_at ? moment(account.merged_at) : '',
          })}
        </>
      )}
      {account.name && (
        <>
          <br />
          <WithName message={w.nameInYup} name={account.name} />
        </>
      )}
    </>
  )

  return (
    <>
      <p>
        <Link to={paths.staff}>{w.back}</Link>
      </p>
      <PageHeading>
        {report.display_code ? fmt(w.detailTitle, { code: report.display_code }) : w.reportLinkNoYup}
      </PageHeading>
      <p className="tags">
        {report.overdue && <span className="tag tag-alert">{w.overdue}</span>}
        <span className="tag">{wording.safety.reasons[report.reason]}</span>
      </p>
      <p className="hint">
        {waiting(report.age_seconds)}
        <br />
        {fmt(w.filed, { date: moment(report.created_at) })}
      </p>
      <dl>
        <dt>{w.details}</dt>
        <dd>{report.details ? <Written>{report.details}</Written> : w.noDetails}</dd>
        <dt>{w.reporter}</dt>
        {/* Every report has a reporter now; only one made through an
            invitation link before reporting needed an account has none. */}
        <dd>{reporter ? who(reporter) : w.reporterLink}</dd>
        {subject && (
          <>
            <dt>{w.subject}</dt>
            <dd>{who(subject)}</dd>
          </>
        )}
      </dl>
      {detail.content_hidden && <p className="notice">{w.contentHidden}</p>}

      <section aria-labelledby="staff-decision">
        <h2 id="staff-decision">{w.decisionHeading}</h2>
        <p>{w.decisionIntro}</p>
        {open ? (
          <NoteAction
            title={w.outcomes[open]}
            text={w.outcomeText[open]}
            required={open !== 'DISMISSED'}
            confirm={w.confirm}
            onConfirm={async (note) => {
              await staffApi.resolve(report.id, open, note)
              decided = open
              navigate(paths.staff)
            }}
            onCancel={() => {
              setOpen(null)
              focusSoon(`decide-${open}`)
            }}
          />
        ) : (
          <div className="actions">
            {OUTCOMES.map((outcome) => (
              <button
                key={outcome}
                type="button"
                id={`decide-${outcome}`}
                onClick={() => setOpen(outcome)}
              >
                {w.outcomes[outcome]}
              </button>
            ))}
          </div>
        )}
      </section>

      {record ? <ReviewedRecord record={record} /> : <p>{w.noRecord}</p>}

      {detail.other_reports.length > 0 && (
        <section aria-labelledby="staff-others">
          <h2 id="staff-others">{w.otherReportsHeading}</h2>
          <ul className="plain">
            {detail.other_reports.map((other) => (
              <li key={other.id} className="card">
                <p>
                  {wording.safety.reasons[other.reason]}
                  {' · '}
                  {other.outcome ? w.outcomeNames[other.outcome] : w.status[other.status]}
                </p>
                <p className="hint">{fmt(w.filed, { date: moment(other.created_at) })}</p>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section aria-labelledby="staff-history">
        <h2 id="staff-history">{w.historyHeading}</h2>
        {detail.history.length === 0 ? (
          <p>{w.historyEmpty}</p>
        ) : (
          <ol className="history">
            {detail.history.map((entry) => (
              <li key={entry.id} className="history-entry">
                <p>{w.actions[entry.action]}</p>
                <p className="hint">
                  {entry.staff_account_id
                    ? fmt(w.byReviewer, { date: moment(entry.at), id: entry.staff_account_id })
                    : fmt(w.byOwner, { date: moment(entry.at) })}
                </p>
                {entry.note && <Written>{entry.note}</Written>}
              </li>
            ))}
          </ol>
        )}
      </section>
    </>
  )
}

/** The reported exchange as recorded: every version sent, and what happened. */
function ReviewedRecord({ record }: { record: NonNullable<ReportDetail['record']> }) {
  const { wording, language } = useI18n()
  const w = wording.staff
  const { exchange, parties } = record
  const when = useMemo(
    () => recordMoments(language, exchange.timezone),
    [language, exchange.timezone],
  )
  const money = useMemo(
    () => moneyIds(record.revisions.map((revision) => termsOfRevision(revision))),
    [record.revisions],
  )

  return (
    <section aria-labelledby="staff-record" className="record">
      <h2 id="staff-record">{w.recordHeading}</h2>
      {!record.complete && <p className="notice">{w.recordIncomplete}</p>}
      {record.revisions.length === 0 && <p>{wording.record.versionsNone}</p>}
      {record.revisions.map((revision) => (
        <Version
          key={revision.id}
          revision={revision}
          name={(slot) => documentOf(revision).parties[slot]}
          when={when}
        />
      ))}
      <h2>{wording.record.eventsHeading}</h2>
      {record.events.length === 0 ? (
        <p>{wording.record.historyEmpty}</p>
      ) : (
        <EventList events={record.events} parties={parties} reader={null} when={when} money={money} />
      )}
    </section>
  )
}
