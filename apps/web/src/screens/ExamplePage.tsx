import type { components } from '@yuppers/api-client'
import {
  deviceTimeZone,
  moveWording,
  sampleYup,
  statusWording,
  type Language,
} from '@yuppers/shared'
import { useEffect, useMemo, useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { EventList } from '../components/EventList'
import { RecordSummary } from '../components/RecordSummary'
import { TermsView } from '../components/TermsView'
import { PageHeading } from '../components/ui'
import { useAnnouncement } from '../lib/announce'

type Contribution = components['schemas']['ContributionDto']

/**
 * The sample yup (DESIGN.md §4.3): a worked example, readable signed out, at
 * `/example` and `/es/example`. It is drawn by the components that draw real
 * yups, from a fixed document with no service behind it, so nothing here is
 * created, saved or sent. Every action is shown disabled, with its own name
 * and a line saying why it does nothing, and a banner says in words, not by
 * colour, that it is an example.
 */
export default function ExamplePage({ language }: { language: string | null }) {
  const i18n = useI18n()
  const { wording, fmt, moment } = i18n
  const { account } = useSession()
  const w = wording.sample

  // A link to `/es/example` is read in Spanish by someone not signed in. A
  // signed-in person keeps their own language, which a link does not change.
  const { setLanguage } = i18n
  useEffect(() => {
    if (language && !account && language !== i18n.language) setLanguage(language as Language)
  }, [language, account, i18n.language, setLanguage])

  // Worked out from today, once, when the page is opened.
  const [now] = useState(() => new Date())
  const sample = useMemo(
    () => sampleYup(now, deviceTimeZone() ?? 'UTC', w, i18n.language),
    [now, w, i18n.language],
  )
  useAnnouncement(w.banner)

  function actionsFor(item: Contribution) {
    const money = sample.money.has(item.id)
    const status = sample.statuses.get(item.id) ?? 'PENDING'
    const provider = item.from === 'A' ? w.firstNameA : w.firstNameB
    // The deposit is done. The others show the step that comes next, disabled.
    const next = status === 'PENDING' ? 'CLAIM' : null
    return (
      <>
        <p className="status">{statusWording(wording, status, money)}</p>
        {next && (
          <>
            <div className="actions">
              <button type="button" disabled aria-describedby={`${item.id}-why`}>
                {moveWording(wording, next, money)}
              </button>
            </div>
            <p className="hint" id={`${item.id}-why`}>
              {w.actionsOff}{' '}
              {item.id === sample.terms.contributions[2].id
                ? w.waitsOn
                : fmt(w.wouldTap, { name: provider })}
            </p>
          </>
        )}
      </>
    )
  }

  return (
    <>
      <PageHeading>{w.title}</PageHeading>
      <p className="notice notice-warning" role="note">
        {w.banner}
      </p>
      <p className="tags">
        <span className="tag">{wording.states.ACTIVE}</span>
      </p>

      <section aria-labelledby="example-terms-heading">
        <h2 id="example-terms-heading">{w.termsHeading}</h2>
        <div className="card">
          <TermsView
            terms={sample.terms}
            currency={sample.currency}
            timezone={sample.timezone}
            you={null}
            statuses={sample.statuses}
            footer={actionsFor}
          />
        </div>
      </section>

      <RecordSummary record={sample.record} />

      <section aria-labelledby="example-history-heading">
        <h2 id="example-history-heading">{w.historyHeading}</h2>
        <EventList
          events={sample.events}
          parties={sample.parties}
          reader={null}
          when={moment}
          money={sample.money}
        />
        <p className="hint">{w.fingerprint}</p>
      </section>

      <div className="actions">
        <Link className="primary button" to={paths.home}>
          {account ? w.startOwn : w.signInToStart}
        </Link>
      </div>
    </>
  )
}
