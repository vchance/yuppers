import type { ExchangeView as Exchange } from '@yuppers/api-client'
import {
  inviteeKind,
  otherPartyName,
  phoneAsTyped,
  type IssuedInvitation,
} from '@yuppers/shared'
import { useEffect, useRef, useState } from 'react'

import { useI18n } from '../app/context'
import { InvitationLink } from '../components/InvitationLink'
import { Notice, PageHeading } from '../components/ui'

interface Props {
  exchange: Exchange
  /** The link just issued with the first proposal, shown once. */
  issued: IssuedInvitation
  /** The person opened a way to send the link. */
  onShared(): void
  /** They are done here, having opened one. */
  onDone(): void
  /** They chose to send it later: the exchange's page reminds them. */
  onLater(): void
}

/**
 * Sending the invitation, as a step of its own after signing (DESIGN.md
 * §8). Yuppers never sends it: the person who signed has to pass the link
 * on, and until they do the other party has nothing. So this is not a panel
 * on the exchange's page to scroll past but the page itself, headed with
 * who the link is for, saying the rule plainly, and leading with the way
 * that reaches them (`ShareActions`). Once a way is opened, the step says
 * what happens next and offers the way on. Not sending is a choice, made
 * with "I’ll send it later", which is as plain as the rest and leaves a
 * reminder on the exchange's page; leaving any other way counts the same.
 */
export function SendInvitation({ exchange, issued, onShared, onDone, onLater }: Props) {
  const { wording, fmt } = useI18n()
  const w = wording.invitationLink
  const name = otherPartyName(exchange) || wording.party.other
  const [shared, setShared] = useState(false)
  const done = useRef<HTMLButtonElement>(null)

  // Once a way to send it is opened, the way on appears, and the keyboard
  // goes to it: what was pressed may have taken the person to another app
  // and back.
  useEffect(() => {
    if (shared) done.current?.focus()
  }, [shared])

  function markShared() {
    setShared(true)
    onShared()
  }

  return (
    <div className="send-step">
      <PageHeading key="send" step name={name}>
        {w.sendTitle}
      </PageHeading>
      <p className="notice notice-warning">{fmt(w.sendRule, { name })}</p>
      <p>
        {/* A US number is written the American way, however it was typed. */}
        {inviteeKind(issued.boundTo) === 'anyone'
          ? w.forAnyoneSummary
          : fmt(w.boundSummary, { identifier: phoneAsTyped(issued.boundTo?.trim() ?? '') })}
      </p>
      <InvitationLink token={issued.token} boundTo={issued.boundTo} onShared={markShared} />
      {shared ? (
        <>
          <Notice>{fmt(w.sharedNotice, { name })}</Notice>
          <div className="actions">
            <button type="button" className="primary" ref={done} onClick={onDone}>
              {w.sendDone}
            </button>
          </div>
        </>
      ) : (
        <div className="send-later">
          <div className="actions">
            <button type="button" className="link" onClick={onLater}>
              {w.later}
            </button>
          </div>
          <p className="hint">{fmt(w.laterHint, { name })}</p>
        </div>
      )}
    </div>
  )
}
