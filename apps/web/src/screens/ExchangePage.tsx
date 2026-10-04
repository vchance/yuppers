import type { ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client'
import type { IssuedInvitation } from '@yuppers/shared'
import { lazy, Suspense, useCallback, useEffect, useState } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { Failure, PageHeading } from '../components/ui'
import { api, failureCode, type RevisionSent } from '../lib/api'
import { ExchangeView } from './ExchangeView'

// Most visits to an exchange are to read it or act on it, not to write terms.
const Composer = lazy(() => import('./Composer'))

/**
 * One exchange, at `/exchanges/{id}`. A draft is its composer; anything
 * further along is the exchange view. `/exchanges/{id}/revise` is the
 * composer again, for a counteroffer or an amendment.
 */
export default function ExchangePage({ id, revising }: { id: string; revising: boolean }) {
  const { wording } = useI18n()
  const [exchange, setExchange] = useState<Exchange | null>(null)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  // The invitation token, held only while this page stays open: it is shown
  // once and cannot be fetched again.
  const [issued, setIssued] = useState<IssuedInvitation | null>(null)

  const reload = useCallback(async () => {
    try {
      const latest = await api.getExchange(id)
      setExchange(latest)
      setFailure(null)
      return latest
    } catch (error) {
      setFailure(failureCode(error))
      return null
    }
  }, [id])

  useEffect(() => {
    let cancelled = false
    api.getExchange(id).then(
      (found) => {
        if (!cancelled) setExchange(found)
      },
      (error: unknown) => {
        if (!cancelled) setFailure(failureCode(error))
      },
    )
    return () => {
      cancelled = true
    }
  }, [id])

  if (!exchange) {
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

  function sent(result: RevisionSent, boundTo: string | null) {
    setExchange(result.exchange)
    setIssued(result.invitation_token ? { token: result.invitation_token, boundTo } : null)
    // Back to the exchange, from the top: the first thing there after a
    // first proposal is the invitation link.
    navigate(paths.exchange(id), { replace: true })
  }

  const composing = exchange.state === 'DRAFT' || revising
  if (composing) {
    return (
      <Suspense fallback={<p>{wording.common.loading}</p>}>
        <Composer exchange={exchange} reload={reload} onSent={sent} />
      </Suspense>
    )
  }

  return (
    <ExchangeView
      exchange={exchange}
      issued={issued}
      onIssued={setIssued}
      onChange={setExchange}
      reload={reload}
    />
  )
}
