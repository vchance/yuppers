import { recordDays, summarizeRecord, summaryText, type RecordDocument } from '@yuppers/shared'
import { useMemo } from 'react'

import { useI18n } from '../app/context'
import { Written } from './ui'

/**
 * The plain summary at the top of a record (DESIGN.md §14.1): who, what each
 * gives, who signed and when, how it stands or ended, and what became of each
 * item. Worked out in the shared package, so the mobile app says the same.
 * Everyone is named: the page may be printed and handed to someone else.
 */
export function RecordSummary({ record }: { record: RecordDocument }) {
  const i18n = useI18n()
  const { language } = i18n
  const { timezone, currency } = record.exchange
  const text = useMemo(
    () => summaryText(summarizeRecord(record), i18n, currency, recordDays(language, timezone)),
    [record, i18n, currency, language, timezone],
  )

  return (
    <section className="card record-plain" aria-labelledby="record-plain-heading">
      <h2 id="record-plain-heading">{text.heading}</h2>
      <p className="hint">{text.intro}</p>
      <p>{text.between}</p>
      <p>{text.basis}</p>
      {text.sides.map((side) => (
        <section key={side.slot} aria-labelledby={`record-plain-${side.slot}`}>
          <h3 id={`record-plain-${side.slot}`}>{side.heading}</h3>
          {side.nothing && <p>{side.nothing}</p>}
          {/* Where a series of payments or stages stands, in words (DESIGN.md §7.1, §7.2). */}
          {side.counts.map((line) => (
            <p key={line} className="series">
              {line}
            </p>
          ))}
          {side.items.length > 0 && (
            <ul className="plain">
              {side.items.map((item) => (
                <li key={item.id} className="contribution">
                  <Written>{item.description}</Written>
                  {item.details.map((detail) => (
                    <p key={detail}>{detail}</p>
                  ))}
                  {item.outcome && <p className="status">{item.outcome}</p>}
                </li>
              ))}
            </ul>
          )}
        </section>
      ))}
      {text.signed.length > 0 && (
        <p>
          {text.signed.map((line, index) => (
            <span key={line} className="record-plain-line">
              {index > 0 && <br />}
              {line}
            </span>
          ))}
        </p>
      )}
      {text.standing.map((line) => (
        <p key={line}>{line}</p>
      ))}
    </section>
  )
}
