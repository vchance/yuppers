import type { components, RevisionTerms } from '@yuppers/api-client'
import { dueDateZone, dueOnDateText, isOverdue, todayIn } from '@yuppers/shared'
import { useMemo, type ReactNode } from 'react'

import { useI18n } from '../app/context'
import type { Slot } from '../lib/api'
import { HelpLink } from './HelpLink'
import { WithName, Written } from './ui'

type Contribution = components['schemas']['ContributionDto']
type Status = components['schemas']['Status']

interface Props {
  terms: RevisionTerms
  currency: string
  /** The exchange's timezone, which its due dates are read in. Unknown before joining. */
  timezone?: string
  /** Which side the reader is; `null` for someone who has not joined yet. */
  you: Slot | null
  /** Where each contribution stands, once the terms are in force. */
  statuses?: ReadonlyMap<string, Status>
  /** Shown under a contribution: its status and what can be done about it. */
  footer?: (contribution: Contribution) => ReactNode
  /**
   * The level of its headings: 3 under a section's own heading, 2 where the
   * terms are the main thing on the page and nothing heads them.
   */
  level?: 2 | 3
}

/**
 * The complete terms of a revision, with nothing collapsed or left for later:
 * this is what a signature covers (DESIGN.md §14.1). Used wherever terms are
 * read, so a proposal looks the same before signing as the agreement does
 * after.
 */
export function TermsView({ terms, currency, timezone, you, statuses, footer, level = 3 }: Props) {
  const H = level === 2 ? 'h2' : 'h3'
  const i18n = useI18n()
  const { wording, fmt, money } = i18n
  const w = wording.terms
  const today = timezone ? todayIn(timezone) : null
  // Named beside each due date when the reader's device keeps another zone.
  const zone = useMemo(() => dueDateZone(timezone), [timezone])
  const nameOf = (slot: Slot) => (slot === 'A' ? terms.party_a_name : terms.party_b_name)
  // Whose side is drawn in the reader's colour. Someone who has not joined
  // yet is reading an invitation to take the invited party's place.
  const coloured = you ?? 'B'

  function due(contribution: Contribution): string {
    const condition = contribution.due
    if (condition.kind === 'DATE') return dueOnDateText(i18n, condition.date, zone)
    if (condition.kind === 'ON_AGREEMENT') return w.dueOnAgreement
    const awaited = terms.contributions.find((other) => other.id === condition.contribution)
    return fmt(w.dueAfter, { description: awaited?.description ?? '' })
  }

  return (
    <div className="terms">
      <p className="hint">{w.ownWords}</p>

      <H>{w.partiesHeading}</H>
      <ul className="plain">
        {(['A', 'B'] as const).map((slot) => (
          <li key={slot}>
            <Written inline>
              {slot === you ? fmt(wording.party.nameYou, { name: nameOf(slot) }) : nameOf(slot)}
            </Written>
          </li>
        ))}
      </ul>

      {terms.terms.trim() !== '' && (
        <>
          <H>{w.termsHeading}</H>
          <Written>{terms.terms}</Written>
        </>
      )}

      {(['A', 'B'] as const).map((slot) => {
        const provided = terms.contributions.filter((contribution) => contribution.from === slot)
        return (
          <section key={slot} className={slot === coloured ? 'party party-you' : 'party'}>
            <H>
              {slot === you ? (
                w.youProvide
              ) : (
                <WithName message={w.otherProvides} name={nameOf(slot)} />
              )}
            </H>
            {provided.length === 0 && <p>{w.nothing}</p>}
            <ul className="plain contributions" hidden={provided.length === 0}>
              {provided.map((contribution) => {
                const status = statuses?.get(contribution.id)
                const overdue =
                  status !== undefined &&
                  today !== null &&
                  isOverdue(status, contribution.due, today)
                return (
                  <li key={contribution.id} className="contribution">
                    <p className="tags">
                      <span className="tag">{wording.contributionTypes[contribution.type]}</span>
                      <span className="tag">{contribution.required ? w.required : w.optional}</span>
                      {overdue && <strong className="tag tag-alert">{w.overdue}</strong>}
                    </p>
                    <Written>{contribution.description}</Written>
                    {contribution.amount_minor != null && (
                      <p>{fmt(w.amount, { amount: money(contribution.amount_minor, currency) })}</p>
                    )}
                    {/* Money is paid outside the product and only recorded here (DESIGN.md §11). */}
                    {contribution.type === 'MONEY' && (
                      <>
                        <p className="hint">{w.moneyOutside}</p>
                        <HelpLink place="moneyOutside" />
                      </>
                    )}
                    {contribution.quantity && (
                      <p>
                        {contribution.quantity.unit
                          ? fmt(w.quantityWithUnit, {
                              amount: contribution.quantity.amount,
                              unit: contribution.quantity.unit,
                            })
                          : fmt(w.quantity, { amount: contribution.quantity.amount })}
                      </p>
                    )}
                    <p>{due(contribution)}</p>
                    {contribution.completion_criteria && (
                      <div className="criteria">
                        <p className="label">{w.criteriaLabel}</p>
                        <Written>{contribution.completion_criteria}</Written>
                      </div>
                    )}
                    {footer?.(contribution)}
                  </li>
                )
              })}
            </ul>
          </section>
        )
      })}

      {timezone && terms.contributions.some((contribution) => contribution.due.kind === 'DATE') && (
        <p className="hint">{fmt(w.timezone, { timezone })}</p>
      )}
    </div>
  )
}
