import type { components } from '@yuppers/api-client'
import {
  documentOf,
  moneyIds,
  recordFile,
  recordMoments,
  statusWording,
  termsOfRevision,
  useRecord,
  verificationText,
  type ClosedReason,
  type RecordDocument,
  type RecordRevision,
} from '@yuppers/shared'
import { useMemo } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { EventList } from '../components/EventList'
import { HelpLink } from '../components/HelpLink'
import { RecordSummary } from '../components/RecordSummary'
import { StatusChip } from '../components/StatusChip'
import { TermsView } from '../components/TermsView'
import { Failure, PageHeading, WithName, Written } from '../components/ui'
import { api } from '../lib/api'
import './record.css'

type Slot = components['schemas']['Slot']

/**
 * The record of an exchange at `/exchanges/{id}/record`: how it stands, every
 * version sent and who signed it, and everything that happened, laid out to
 * be read and printed, under a plain summary of it. "Save as PDF" opens the
 * browser's print window, which makes the PDF; the download is the same
 * record as a file (DESIGN.md §14.1).
 */
export default function RecordPage({ id }: { id: string }) {
  const { wording } = useI18n()
  // A long record comes in parts; the page shows it whole.
  const { record, failure } = useRecord(api, id)

  if (record) return <Record record={record} />
  if (!failure) return <p>{wording.common.loading}</p>
  return (
    <>
      <PageHeading>{wording.exchange.titleNoName}</PageHeading>
      <Failure code={failure} />
      <p>
        <Link to={paths.home}>{wording.common.goHome}</Link>
      </p>
    </>
  )
}

/** Hands the record to the browser as a file to keep. */
function download(record: RecordDocument, name: string) {
  const copy = recordFile(record, name)
  const file = new Blob([copy.text], { type: copy.type })
  const address = URL.createObjectURL(file)
  const link = document.createElement('a')
  link.href = address
  link.download = copy.name
  document.body.append(link)
  link.click()
  link.remove()
  // After the browser has had the chance to start saving it.
  window.setTimeout(() => URL.revokeObjectURL(address), 1000)
}

function Record({ record }: { record: RecordDocument }) {
  const { wording, fmt, language } = useI18n()
  const w = wording.record
  const { exchange, parties } = record
  const when = useMemo(() => recordMoments(language, exchange.timezone), [language, exchange.timezone])
  const code = exchange.display_code
  const reason = exchange.closed_reason
  // Money is spoken of in words for paying and receiving (DESIGN.md §11).
  const money = useMemo(() => moneyIds(record.revisions.map(termsOfRevision)), [record])

  return (
    // Everyone is named here, the reader included: the page may be printed
    // and handed to someone who is neither party.
    <div className="record-page">
      <PageHeading>{fmt(w.title, { code })}</PageHeading>
      <div className="actions no-print">
        <Link className="button" to={paths.exchange(exchange.id)}>
          {w.back}
        </Link>
        {/* The PDF is the browser's: its print window saves the page as one,
            and the print rules in record.css leave only the record on it. */}
        <button type="button" className="primary" onClick={() => window.print()}>
          {w.summary.savePdf}
        </button>
        <button type="button" onClick={() => download(record, fmt(w.fileName, { code }))}>
          {w.download}
        </button>
      </div>
      <p className="hint no-print">{w.summary.savePdfHint}</p>
      <HelpLink place="record" />
      <p className="hint">
        {fmt(w.madeFor, { name: parties[record.prepared_for], date: when(record.generated_at) })}
        <br />
        {fmt(w.timesIn, { timezone: exchange.timezone })}
      </p>
      {/* A reviewer has hidden what the parties wrote from this reader (DESIGN.md §9). */}
      {record.content_hidden && <p className="notice notice-warning">{w.contentHidden}</p>}

      <RecordSummary record={record} />

      <section aria-labelledby="record-summary">
        <h2 id="record-summary">{w.summaryHeading}</h2>
        <p className="tags">
          <span className="tag">
            {exchange.closed_outcome
              ? wording.outcomes[exchange.closed_outcome]
              : wording.states[exchange.state]}
          </span>
          <span className="tag">{fmt(wording.home.reference, { code })}</span>
        </p>
        <h3>{wording.terms.partiesHeading}</h3>
        <ul className="plain">
          {(['A', 'B'] as const).map(
            (slot) =>
              parties[slot] && (
                <li key={slot}>
                  <Written inline>{parties[slot]}</Written>
                </li>
              ),
          )}
        </ul>
        {reason && Object.hasOwn(w.closedReasons, reason) && <p>{w.closedReasons[reason as ClosedReason]}</p>}
        <p>
          {fmt(w.started, { date: when(exchange.created_at) })}
          {exchange.closed_at && (
            <>
              <br />
              {fmt(w.closedOn, { date: when(exchange.closed_at) })}
            </>
          )}
        </p>
        <p>
          {exchange.in_force_revision
            ? fmt(w.agreementIs, { number: exchange.in_force_revision.sequence })
            : w.agreementNone}
        </p>
        {exchange.open_revision && (
          <p>{fmt(w.waiting, { number: exchange.open_revision.sequence })}</p>
        )}
        {exchange.end_proposed_by && (
          <p>{fmt(w.endProposed, { name: parties[exchange.end_proposed_by] })}</p>
        )}
        {exchange.close_requested_by && exchange.close_requested_at && (
          <p>
            {fmt(w.closeRequested, {
              name: parties[exchange.close_requested_by],
              date: when(exchange.close_requested_at),
            })}
          </p>
        )}
      </section>

      <section aria-labelledby="record-about">
        <h2 id="record-about">{w.aboutHeading}</h2>
        <p>{w.export.about}</p>
        <p>{w.export.signatures}</p>
        <p>{w.export.statements}</p>
        <p>{w.export.contentHash}</p>
      </section>

      {record.contributions.length > 0 && (
        <section aria-labelledby="record-items">
          <h2 id="record-items">{w.itemsHeading}</h2>
          <ul className="plain">
            {record.contributions.map((contribution) => (
              <li key={contribution.id} className="contribution">
                <Written>{contribution.description}</Written>
                <p>{fmt(w.itemFrom, { name: parties[contribution.from] })}</p>
                <p className="status">
                  <StatusChip status={contribution.status}>
                    {statusWording(wording, contribution.status, money.has(contribution.id))}
                  </StatusChip>
                </p>
                {contribution.since && (
                  <p className="hint">{fmt(w.since, { date: when(contribution.since) })}</p>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {record.revisions.length === 0 && <p>{w.versionsNone}</p>}
      {record.revisions.map((revision) => (
        <Version
          key={revision.id}
          revision={revision}
          name={(slot) => documentOf(revision).parties[slot]}
          when={when}
        />
      ))}

      <section aria-labelledby="record-events">
        <h2 id="record-events">{w.eventsHeading}</h2>
        {record.events.length === 0 ? (
          <p>{w.historyEmpty}</p>
        ) : (
          <EventList
            events={record.events}
            parties={parties}
            reader={null}
            when={when}
            money={money}
          />
        )}
      </section>

      {record.history_chain && <p className="hint">{record.history_chain}</p>}
    </div>
  )
}

interface VersionProps {
  revision: RecordRevision
  /** A party's name as this version writes it. */
  name(slot: Slot): string
  when(instant: string): string
}

/**
 * One version that was sent: who sent it and what became of it, its complete
 * terms, the fingerprint a signature covers, and each signature with what it
 * rests on.
 */
export function Version({ revision, name, when }: VersionProps) {
  const { wording, fmt } = useI18n()
  const w = wording.record
  const { standing } = revision
  const signed = documentOf(revision)
  const heading = `record-version-${revision.sequence}`

  return (
    <section className="card record-version" aria-labelledby={heading}>
      <div className="record-version-head">
        <h2 id={heading}>{fmt(w.versionHeading, { number: revision.sequence })}</h2>
        <p>
          <WithName
            message={w.versionSent}
            name={name(revision.author)}
            values={{ date: when(revision.sent_at), expires: when(revision.expires_at) }}
          />
          {revision.answers && (
            <>
              <br />
              {fmt(w.versionAnswers, { number: revision.answers.sequence })}
            </>
          )}
        </p>
        <p className="status">{w.versionStatus[standing.status]}</p>
        <p className="hint">
          {fmt(w.since, { date: when(standing.since) })}
          {standing.replaced_by && (
            <>
              <br />
              {fmt(w.versionReplacedBy, { number: standing.replaced_by.sequence })}
            </>
          )}
        </p>
      </div>

      {revision.note && (
        <>
          <h3>{w.noteLabels.message}</h3>
          <Written>{revision.note}</Written>
        </>
      )}

      <TermsView
        terms={termsOfRevision(revision)}
        currency={signed.currency}
        timezone={signed.timezone}
        you={null}
      />
      <p className="hint fingerprint">
        {fmt(wording.terms.fingerprint, { hash: revision.content_hash })}
      </p>

      <h3>{w.signaturesHeading}</h3>
      <ul className="plain">
        {revision.signatures.map((signature) => (
          <li key={signature.party} className="record-signature">
            <p>
              <WithName
                message={w.versionSignedBy}
                name={signature.name}
                values={{ date: when(signature.signed_at) }}
              />
            </p>
            <p className="hint">
              {fmt(w.verifiedBy, {
                method: verificationText(signature.verification, w.export.verification),
              })}
              <br />
              {fmt(w.verifiedAt, { date: when(signature.verification.verified_at) })}
              {signature.verification.attribution && (
                <>
                  <br />
                  {signature.verification.attribution}
                </>
              )}
              <br />
              {fmt(w.consentShown, {
                version: signature.consent.version,
                language: signature.consent.language,
              })}
            </p>
          </li>
        ))}
        {/* Left by someone who opened the invitation and was removed, or
            left, before being confirmed (DESIGN.md §8). Kept because it
            happened; it names nobody and counts for nothing. */}
        {revision.void_signatures?.map((signature) => (
          <li key={`void-${signature.signed_at}`} className="record-signature">
            <p>
              {fmt(wording.claimant.voidSignature, {
                date: when(signature.signed_at),
                since: when(signature.void_since),
              })}
            </p>
            <p className="hint">
              {fmt(w.verifiedBy, {
                method: verificationText(signature.verification, w.export.verification),
              })}
              <br />
              {fmt(w.verifiedAt, { date: when(signature.verification.verified_at) })}
              {signature.verification.attribution && (
                <>
                  <br />
                  {signature.verification.attribution}
                </>
              )}
              <br />
              {fmt(w.consentShown, {
                version: signature.consent.version,
                language: signature.consent.language,
              })}
            </p>
          </li>
        ))}
      </ul>
    </section>
  )
}
