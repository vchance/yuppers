import type { ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client'
import {
  applyTemplate,
  deviceTimeZone,
  pendingStart,
  templateById,
  type IssuedInvitation,
  type PendingStart,
} from '@yuppers/shared'
import { lazy, Suspense, useCallback, useEffect, useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { fresh } from '../app/fresh'
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
export default function ExchangePage({
  id,
  revising,
  from,
}: {
  /** Null for a fresh start, which has no exchange until the person changes something. */
  id: string | null
  revising: boolean
  /** A fresh start's choice: a template's id, or `blank`. */
  from?: string
}) {
  const { wording } = useI18n()
  const { account } = useSession()
  // A fresh start is a local working copy; the exchange is made on the first
  // change, and the address then moves to it (DESIGN.md §4.4).
  const [pending] = useState<PendingStart | undefined>(() => {
    if (!from) return undefined
    const timezone = deviceTimeZone() ?? 'UTC'
    const template = from === 'blank' ? undefined : templateById(from)
    if (!template) return pendingStart(api, timezone, { kind: 'blank' }, null)
    const draft = applyTemplate(
      template,
      wording.templates.entries[template.id],
      account?.display_name ?? '',
      () => crypto.randomUUID(),
    )
    return pendingStart(api, timezone, { kind: 'template', template }, draft)
  })
  const [exchange, setExchange] = useState<Exchange | null>(pending?.exchange ?? null)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  // The invitation token, held only while this page stays open: it is shown
  // once and cannot be fetched again.
  const [issued, setIssued] = useState<IssuedInvitation | null>(null)
  // Whether the step that sends the link is the page: from sending a first
  // proposal until the person opens a way to send it and goes on, or says
  // they will send it later.
  const [sending, setSending] = useState(false)

  const reload = useCallback(async () => {
    const at = pending?.created()?.id ?? id
    if (!at) return null
    try {
      const latest = await api.getExchange(at)
      setExchange(latest)
      setFailure(null)
      return latest
    } catch (error) {
      setFailure(failureCode(error))
      return null
    }
  }, [id, pending])

  useEffect(() => {
    // A fresh start has nothing to read; one just made is already in hand.
    if (!id || pending?.created()?.id === id) return
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
  }, [id, pending])

  // The person opened a way to send the link. The service is told, so the
  // reminder on this page and the chip in the list know; the page itself
  // knows at once, and a request that fails changes nothing it can show.
  const shared = useCallback(() => {
    const at = new Date().toISOString()
    setExchange((current) => current && { ...current, invitation_shared_at: at })
    api.markInvitationShared(exchange?.id ?? id ?? '').catch(() => {})
  }, [id, exchange?.id])

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
    navigate(paths.exchange(result.exchange.id), { replace: true })
  }

  // The exchange of a fresh start has just been made: this page is now that
  // exchange's, at its own address, replacing the one that had none.
  function created(made: Exchange) {
    setExchange((current) => (current?.id === made.id ? current : made))
    fresh.id = made.id
    if (window.location.pathname !== paths.exchange(made.id)) {
      navigate(paths.exchange(made.id), { replace: true })
    }
  }

  const composing = exchange.state === 'DRAFT' || revising
  if (composing) {
    return (
      <Suspense fallback={<p>{wording.common.loading}</p>}>
        <Composer
          exchange={exchange}
          reload={reload}
          onSent={sent}
          pending={pending}
          onCreated={created}
        />
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
