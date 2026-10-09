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
import { SendInvitation } from './SendInvitation'

// Most visits to an exchange are to read it or act on it, not to write terms.
const Composer = lazy(() => import('./Composer'))

/**
 * One exchange, at `/exchanges/{id}`. A draft is its composer; a first
 * proposal just sent is the step that sends its invitation link; anything
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
  // Whether the step that sends the link is the page: from sending a first
  // proposal until the person opens a way to send it and goes on, or says
  // they will send it later.
  const [sending, setSending] = useState(false)

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

  // The person opened a way to send the link. The service is told, so the
  // reminder on this page and the chip in the list know; the page itself
  // knows at once, and a request that fails changes nothing it can show.
  const shared = useCallback(() => {
    const at = new Date().toISOString()
    setExchange((current) => current && { ...current, invitation_shared_at: at })
    api.markInvitationShared(id).catch(() => {})
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
    const link = result.invitation_token ? { token: result.invitation_token, boundTo } : null
    setIssued(link)
    // A first proposal is not done until its link is sent: that step comes
    // next, as the page, before the exchange itself is shown.
    setSending(link !== null)
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

  if (sending && issued) {
    return (
      <SendInvitation
        exchange={exchange}
        issued={issued}
        onShared={shared}
        onDone={() => setSending(false)}
        onLater={() => setSending(false)}
      />
    )
  }

  return (
    <ExchangeView
      exchange={exchange}
      issued={issued}
      onIssued={setIssued}
      onShared={shared}
      onChange={setExchange}
      reload={reload}
    />
  )
}
