import type { ErrorCode, ExchangeSummary } from '@yuppers/api-client'
import { groupExchanges, invitationChip } from '@yuppers/shared'
import { useEffect, useState } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { CombinedNotice } from '../components/CombinedNotice'
import { Mark } from '../components/Mark'
import { StatusChip } from '../components/StatusChip'
import { ErrorNote, Failure, PageHeading, Written } from '../components/ui'
import { api, failureCode } from '../lib/api'

import '../components/brand.css'

/**
 * The signed-in person's exchanges and the way to start one. What is in
 * progress comes first, since that is what may be waiting on them; drafts
 * they never sent come next; what is closed is kept but folded away, so it
 * never buries the rest. Within a group, most recently changed first.
 *
 * With no exchanges yet, the page is the mark, what to do, and the way to
 * start, in one place rather than a button over an empty list.
 */
export default function HomePage() {
  const { wording, fmt } = useI18n()
  const w = wording.home

  const [exchanges, setExchanges] = useState<ExchangeSummary[] | null>(null)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [starting, setStarting] = useState(false)
  const [tooMany, setTooMany] = useState(false)
  const [showClosed, setShowClosed] = useState(false)

  useEffect(() => {
    let cancelled = false
    api.listExchanges().then(
      (found) => {
        if (!cancelled) setExchanges(found)
      },
      (error: unknown) => {
        if (!cancelled) setFailure(failureCode(error))
      },
    )
    return () => {
      cancelled = true
    }
  }, [])

  async function start() {
    setStarting(true)
    setFailure(null)
    setTooMany(false)
    try {
      // Due dates are read in the timezone of whoever starts the exchange.
      const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone
      const exchange = await api.createExchange(timezone)
      navigate(paths.exchange(exchange.id))
    } catch (error) {
      const code = failureCode(error)
      // Here the limit is on exchanges started today, not on codes.
      if (code === 'TOO_MANY_REQUESTS') setTooMany(true)
      else setFailure(code)
      setStarting(false)
    }
  }

  const groups = exchanges ? groupExchanges(exchanges) : null
  const empty = exchanges?.length === 0
  const startButton = (
    <div className="actions">
      <button type="button" className="primary" disabled={starting} onClick={start}>
        {w.start}
      </button>
    </div>
  )

  return (
    <>
      <PageHeading>{w.title}</PageHeading>
      <CombinedNotice />
      {!empty && startButton}
      <Failure code={failure} />
      {tooMany && <ErrorNote>{w.tooManyToday}</ErrorNote>}

      {!exchanges && !failure && <p>{wording.common.loading}</p>}
      {empty && (
        <div className="empty-state">
          <Mark />
          <p>{w.empty}</p>
          {startButton}
        </div>
      )}
      {groups && (
        <>
          <Group id="open" heading={w.groupOpen} exchanges={groups.open} />
          <Group id="drafts" heading={w.groupDrafts} exchanges={groups.drafts} />
          {groups.closed.length > 0 && (
            <section aria-labelledby="exchanges-closed">
              <h2 id="exchanges-closed">{w.groupClosed}</h2>
              <div className="actions">
                <button
                  type="button"
                  aria-expanded={showClosed}
                  onClick={() => setShowClosed((shown) => !shown)}
                >
                  {showClosed ? w.hideClosed : fmt(w.showClosed, { count: groups.closed.length })}
                </button>
              </div>
              {showClosed && <Cards exchanges={groups.closed} />}
            </section>
          )}
        </>
      )}
    </>
  )
}

function Group({
  id,
  heading,
  exchanges,
}: {
  id: string
  heading: string
  exchanges: readonly ExchangeSummary[]
}) {
  if (exchanges.length === 0) return null
  return (
    <section aria-labelledby={`exchanges-${id}`}>
      <h2 id={`exchanges-${id}`}>{heading}</h2>
      <Cards exchanges={exchanges} />
    </section>
  )
}

function Cards({ exchanges }: { exchanges: readonly ExchangeSummary[] }) {
  const { wording, fmt, moment } = useI18n()
  const w = wording.home
  return (
    <ul className="plain cards">
      {exchanges.map((exchange) => {
        // The initiator holds the link that brings the other party in, and
        // Yuppers never sends it: until someone joins, the card says whether
        // they have sent it (`invitationChip`), on a chip like an item's status.
        const chip = invitationChip(exchange)
        return (
          <li key={exchange.id} className="card">
            <Link to={paths.exchange(exchange.id)} className="card-link">
              {exchange.other_party_name ? (
                <Written inline>{fmt(w.withParty, { name: exchange.other_party_name })}</Written>
              ) : (
                w.noParty
              )}
            </Link>
            <p className="tags">
              <span className="tag">
                {exchange.closed_outcome
                  ? wording.outcomes[exchange.closed_outcome]
                  : wording.states[exchange.state]}
              </span>
              {chip === 'notSent' && <StatusChip status="PENDING">{w.notSent}</StatusChip>}
              {chip === 'waiting' && (
                <StatusChip status="CLAIMED">
                  {fmt(w.waitingFor, { name: exchange.other_party_name })}
                </StatusChip>
              )}
            </p>
            <p className="hint">
              {fmt(w.reference, { code: exchange.display_code })}
              <br />
              {fmt(w.updated, { date: moment(exchange.updated_at) })}
            </p>
          </li>
        )
      })}
    </ul>
  )
}
