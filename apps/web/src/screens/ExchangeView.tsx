import type { ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client'
import {
  boundToProblemText,
  consentShown,
  invitationBoundTo,
  invitationForProblem,
  isInvitationSpent,
  isUnconfirmedClaimant,
  moneyIds,
  NAMED_INVITATION,
  otherPartyName,
  paymentOptionsKey,
  remainingRequired,
  sendReminder,
  statusesOf,
  troublePanel,
  troubleSituationOf,
  useHistory,
  useSignInChannels,
  type ClosedReason,
  type InvitationChoice,
  type IssuedInvitation,
} from '@yuppers/shared'
import {
  lazy,
  Suspense,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { Consent } from '../components/Consent'
import { InvitationFor, InvitationLink } from '../components/InvitationLink'
import { Mark } from '../components/Mark'
import { ShowWhenSigning } from '../components/ShowWhenSigning'
import { OtherPartyLeft } from '../components/OtherPartyLeft'
import { Panel } from '../components/Panel'
import { TermsView } from '../components/TermsView'
import { WalletButton } from '../components/WalletButton'
import { Failure, Notice, PageHeading, WithName, Written } from '../components/ui'
import { restoreFocus, useActions, type Actions } from '../lib/actions'
import { useAnnouncement } from '../lib/announce'
import { focusLost } from '../lib/focus'
import { showAfterSigning } from '../lib/payments'
import { api, failureCode, type RevisionView } from '../lib/api'
import { ClaimantWaiting, ConfirmClaimant } from './Claimant'
import { Ending } from './Ending'
import { ExchangeSafety } from './ExchangeSafety'
import { Fulfillment } from './Fulfillment'
import { History } from './History'
import { ProposalChanges } from './ProposalChanges'
import { Trouble } from './Trouble'

// Text updates are for parties to an agreement, never needed by the
// invitation page, which has a size budget (`scripts/check-budget.mjs`).
const SmsUpdates = lazy(() =>
  import('../components/SmsUpdates').then((module) => ({ default: module.SmsUpdates })),
)
// Likewise payment options, for a party who receives money.
const ShowPaymentOptions = lazy(() =>
  import('../components/ShowPaymentOptions').then((module) => ({
    default: module.ShowPaymentOptions,
  })),
)

/** How often an open exchange is checked for what the other party has done. */
const CHECK_EVERY_MS = 20_000

interface Props {
  exchange: Exchange
  /** A just-issued invitation, to show once. */
  issued: IssuedInvitation | null
  onIssued(issued: IssuedInvitation | null): void
  /** The initiator opened a way to send the link they hold. */
  onShared(): void
  onChange(exchange: Exchange): void
  reload(): Promise<Exchange | null>
}

/**
 * An exchange as one of its two parties sees it: its state, the revision
 * waiting to be signed, the agreement in force and where each contribution
 * stands, and every action open to this party right now. The service decides
 * what is allowed; this offers what should be, and shows the refusal if it
 * was wrong.
 */
export function ExchangeView({ exchange, issued, onIssued, onShared, onChange, reload }: Props) {
  const { wording, fmt } = useI18n()
  const w = wording.exchange
  const actions = useActions(exchange, onChange, reload)

  const you = exchange.you
  const open = exchange.open_revision ?? null
  const inForce = exchange.in_force_revision ?? null
  // Read here rather than in the history section: an exchange closed without
  // agreement has no revision to read names from, and the history names the
  // parties as the last terms did.
  const history = useHistory(api, exchange)
  const writtenName = otherPartyName(exchange, history.page?.parties)
  // In a sentence, someone with no name yet is "the other party".
  const otherName = writtenName || wording.party.other
  const active = exchange.state === 'ACTIVE'
  // Money is spoken of in words for paying and receiving (DESIGN.md §11).
  const money = useMemo(() => moneyIds([open?.terms, inForce?.terms]), [open, inForce])

  // A newer version found while the person is in the middle of something is
  // held back and offered, not swapped in under them.
  const [newer, setNewer] = useState<Exchange | null>(null)
  const [refreshed, setRefreshed] = useState(false)
  const current = useRef({ exchange, engaged: false })
  useEffect(() => {
    current.current = { exchange, engaged: actions.panel !== null || actions.busy }
  })

  useEffect(() => {
    if (exchange.state === 'CLOSED') return
    let cancelled = false
    async function check() {
      if (document.visibilityState !== 'visible' || current.current.engaged) return
      let found: Exchange
      try {
        found = await api.getExchange(exchange.id)
      } catch {
        return
      }
      // Payment options change no version: a payee who stops showing them
      // is noticed too (`paymentOptionsKey`).
      if (
        cancelled ||
        (found.version === current.current.exchange.version &&
          paymentOptionsKey(found) === paymentOptionsKey(current.current.exchange))
      )
        return
      if (current.current.engaged) setNewer(found)
      else {
        onChange(found)
        setRefreshed(true)
      }
    }
    const timer = window.setInterval(check, CHECK_EVERY_MS)
    document.addEventListener('visibilitychange', check)
    return () => {
      cancelled = true
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', check)
    }
  }, [exchange.id, exchange.state, onChange])

  // A refusal with no panel left to show it in, such as one that reloaded the
  // exchange, is shown at the top, and the top is brought into view: the
  // person may be far down the page, looking at what they just pressed.
  const refused = actions.panel === null ? actions.failure : null
  const refusal = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (refused) refusal.current?.scrollIntoView({ block: 'center' })
  }, [refused])

  // After an action has taken effect its panel is gone, and often the button
  // that opened it too. The keyboard goes back to that button if it is still
  // there, and otherwise to the notice that says the exchange changed.
  const updated = useRef<HTMLParagraphElement>(null)
  const done = actions.done
  useEffect(() => {
    if (!done || !focusLost()) return
    if (!restoreFocus()) updated.current?.focus()
  }, [done])
  useAnnouncement(newer ? w.newer : null)
  useAnnouncement((actions.done || refreshed) && !newer ? w.updated : null)

  const statuses = statusesOf(exchange)
  const since = new Map(exchange.contributions.map((item) => [item.id, item.since ?? null]))
  const remaining = remainingRequired(exchange)

  return (
    <>
      {writtenName ? (
        <PageHeading name={writtenName}>{w.title}</PageHeading>
      ) : (
        <PageHeading>{w.titleNoName}</PageHeading>
      )}
      <p className="tags">
        <span className="tag">
          {exchange.closed_outcome
            ? wording.outcomes[exchange.closed_outcome]
            : wording.states[exchange.state]}
        </span>
        <span className="tag">{fmt(wording.home.reference, { code: exchange.display_code })}</span>
      </p>
      {exchange.closed_reason && Object.hasOwn(wording.closedReasons, exchange.closed_reason) && (
        <p>{wording.closedReasons[exchange.closed_reason as ClosedReason]}</p>
      )}

      <div ref={refusal}>
        <Failure code={refused} />
      </div>
      {(actions.done || refreshed) && !newer && (
        <p className="notice" tabIndex={-1} ref={updated}>
          {w.updated}
        </p>
      )}
      {newer && (
        <div className="notice">
          <p>{w.newer}</p>
          <button
            type="button"
            onClick={() => {
              actions.close()
              onChange(newer)
              setNewer(null)
            }}
          >
            {w.showLatest}
          </button>
        </div>
      )}

      <OtherPartyLeft exchange={exchange} otherName={otherName} />
      {/* A reviewer has hidden what the parties wrote from this reader (DESIGN.md §9). */}
      {exchange.content_hidden && (
        <p className="notice notice-warning">{wording.exchange.contentHidden}</p>
      )}
      <Counterparty
        exchange={exchange}
        otherName={otherName}
        actions={actions}
        issued={issued}
        onIssued={onIssued}
        onShared={onShared}
        reload={reload}
      />

      {open && (
        <OpenRevision
          exchange={exchange}
          revision={open}
          otherName={otherName}
          actions={actions}
          reload={reload}
        />
      )}

      {inForce && (
        <section className="card" aria-labelledby="agreement-heading">
          <h2 id="agreement-heading">{w.agreementHeading}</h2>
          {/* Where the two meet it turns green: the one moment that moves,
              settling straight as it appears (`.agreed` in index.css). */}
          <div className="agreed">
            <Mark check />
            <p className="agreed-text">{w.agreementSigned}</p>
          </div>
          {active && remaining > 0 && <p>{fmt(w.remaining, { count: remaining })}</p>}
          {active && <WalletButton exchange={exchange} />}
          <TermsView
            terms={inForce.terms}
            currency={exchange.currency}
            timezone={exchange.timezone}
            you={you}
            statuses={statuses}
            footer={(contribution) => (
              <Fulfillment
                contribution={contribution}
                status={statuses.get(contribution.id) ?? 'PENDING'}
                since={since.get(contribution.id) ?? null}
                you={you}
                otherName={otherName}
                active={active}
                actions={actions}
                exchange={exchange}
                onChange={onChange}
              />
            )}
          />
          <p className="hint fingerprint">
            {fmt(wording.terms.fingerprint, { hash: inForce.content_hash })}
          </p>
          {active && (
            <div className="actions">
              {!open && (
                <Link className="button" to={paths.revise(exchange.id)}>
                  {w.amend}
                </Link>
              )}
              {/* One way in to the ways out (DESIGN.md §5.3). */}
              <button
                type="button"
                aria-expanded={troubleSituationOf(actions.panel) !== undefined}
                disabled={actions.busy}
                onClick={() => actions.open(troublePanel())}
              >
                {wording.trouble.open}
              </button>
            </div>
          )}
          {active && troubleSituationOf(actions.panel) !== undefined && (
            <Trouble
              key={actions.panel}
              exchange={exchange}
              otherName={otherName}
              actions={actions}
            />
          )}
        </section>
      )}

      {active && <Ending exchange={exchange} otherName={otherName} actions={actions} />}

      <Suspense fallback={null}>
        <ShowPaymentOptions exchange={exchange} otherName={otherName} reload={reload} />
      </Suspense>

      <Suspense fallback={null}>
        <SmsUpdates exchange={exchange} />
      </Suspense>

      <History exchange={exchange} reading={history} money={money} />

      <ExchangeSafety exchange={exchange} otherName={otherName} actions={actions} reload={reload} />

      {exchange.state !== 'CLOSED' && (
        <div className="actions">
          <button
            type="button"
            className="link"
            onClick={() => {
              setRefreshed(false)
              void reload()
            }}
          >
            {w.refresh}
          </button>
        </div>
      )}
    </>
  )
}

interface CounterpartyProps {
  exchange: Exchange
  otherName: string
  actions: Actions
  issued: IssuedInvitation | null
  onIssued(issued: IssuedInvitation | null): void
  onShared(): void
  reload(): Promise<Exchange | null>
}

/**
 * Who is on the other side (DESIGN.md §8). Until someone opens the link the
 * initiator is reminded to send it, and can replace it; once someone has,
 * the initiator confirms it is who they meant before any signature takes
 * effect, or removes them and makes a new link.
 */
function Counterparty({
  exchange,
  otherName,
  actions,
  issued,
  onIssued,
  onShared,
  reload,
}: CounterpartyProps) {
  const { wording, fmt, moment } = useI18n()
  const link = wording.invitationLink
  const initiator = exchange.you === 'A'
  const claimant = exchange.claimant ?? null
  // Set when the initiator has just removed whoever opened the link, so the
  // way to a new link can say why it is being offered.
  const [removed, setRemoved] = useState(false)

  if (exchange.state !== 'NEGOTIATING') return null

  if (initiator && exchange.counterparty === 'UNCLAIMED') {
    // The link was used by someone who is gone again, or ran out: there is
    // none to lose or to have sent to the wrong person, only one to make.
    const spent = isInvitationSpent(exchange) && !issued
    // Yuppers never sends the link, so while nobody has joined the card is a
    // reminder to send it: at once if no way to send it was ever opened,
    // and again once that was long enough ago (`sendReminder`).
    const reminder = spent ? null : sendReminder(exchange)
    const sharedAt = exchange.invitation_shared_at ?? null
    return (
      <section
        className={reminder ? 'card card-reminder' : 'card'}
        aria-labelledby="invitation-heading"
      >
        <h2 id="invitation-heading">
          <WithName message={link.notJoined} name={otherName} />
        </h2>
        {removed && !issued && <Notice>{wording.claimant.rejected}</Notice>}
        {spent && <p>{wording.claimant.linkUsed}</p>}
        {reminder === 'unsent' && (
          <p className="notice notice-warning">{fmt(link.notSentYet, { name: otherName })}</p>
        )}
        {reminder === 'waiting' && sharedAt && (
          <p className="notice notice-warning">
            {fmt(link.sharedLongAgo, { date: moment(sharedAt), name: otherName })}
          </p>
        )}
        {!reminder && !spent && sharedAt && <p>{fmt(link.sharedOn, { date: moment(sharedAt) })}</p>}
        {!reminder && !spent && <p>{link.unclaimed}</p>}
        {issued && (
          <InvitationLink
            key={issued.token}
            token={issued.token}
            boundTo={issued.boundTo}
            onShared={onShared}
          />
        )}
        {!spent && <p>{!issued && reminder ? link.sendAgain : link.reissueIntro}</p>}
        <div className="actions">
          <button
            type="button"
            aria-expanded={actions.panel === 'reissue'}
            onClick={() => actions.open('reissue')}
          >
            {link.reissue}
          </button>
        </div>
        {actions.panel === 'reissue' && (
          <Reissue exchange={exchange.id} actions={actions} onIssued={onIssued} reload={reload} />
        )}
      </section>
    )
  }

  if (initiator && exchange.counterparty === 'CLAIMED' && claimant) {
    return (
      <ConfirmClaimant
        exchange={exchange}
        claimant={claimant}
        actions={actions}
        onRejected={() => {
          // Straight on to making a link for the person who was meant.
          setRemoved(true)
          actions.open('reissue')
        }}
      />
    )
  }

  if (isUnconfirmedClaimant(exchange)) {
    return <ClaimantWaiting exchange={exchange} otherName={otherName} actions={actions} />
  }
  return null
}

interface ReissueProps {
  exchange: string
  actions: Actions
  onIssued(issued: IssuedInvitation | null): void
  reload(): Promise<Exchange | null>
}

function Reissue({ exchange, actions, onIssued, reload }: ReissueProps) {
  const { wording, fmt } = useI18n()
  const link = wording.invitationLink
  const forId = useId()
  const [invitee, setInvitee] = useState<InvitationChoice>(NAMED_INVITATION)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [checked, setChecked] = useState(false)
  const channels = useSignInChannels(api)
  const problem = checked ? invitationForProblem(invitee, channels) : null

  async function submit(event: FormEvent) {
    event.preventDefault()
    setChecked(true)
    if (invitationForProblem(invitee, channels)) {
      window.setTimeout(() => document.getElementById(forId)?.focus())
      return
    }
    setBusy(true)
    setFailure(null)
    const bound = invitationBoundTo(invitee)
    try {
      onIssued({ token: await api.reissueInvitation(exchange, bound), boundTo: bound })
      actions.close()
    } catch (error) {
      const code = failureCode(error)
      setFailure(code)
      // Refused because someone has opened the link in the meantime.
      if (code === 'ACTION_NOT_ALLOWED') await reload()
    } finally {
      setBusy(false)
    }
  }

  return (
    <Panel title={link.reissue}>
      <form noValidate onSubmit={submit}>
        <InvitationFor
          choice={invitee}
          onChange={setInvitee}
          channels={channels}
          id={forId}
          error={problem ? boundToProblemText(problem, link, channels, fmt) : null}
        />
        <Failure code={failure} />
        <div className="actions">
          <button type="submit" className="primary" disabled={busy}>
            {link.reissue}
          </button>
          <button type="button" disabled={busy} onClick={actions.close}>
            {wording.common.cancel}
          </button>
        </div>
      </form>
    </Panel>
  )
}

interface OpenRevisionProps {
  exchange: Exchange
  revision: RevisionView
  otherName: string
  actions: Actions
  reload(): Promise<Exchange | null>
}

/**
 * The one revision waiting to be signed: a first proposal, a counteroffer,
 * or an amendment to the agreement in force. Its author signed it by sending
 * it; the other party can sign it, decline it, or answer with their own.
 */
function OpenRevision({ exchange, revision, otherName, actions, reload }: OpenRevisionProps) {
  const { wording, fmt, moment, language } = useI18n()
  const w = wording.exchange
  const you = exchange.you
  const yours = revision.author === you
  const youSigned = revision.accepted_by.includes(you)
  const otherSigned = revision.accepted_by.some((slot) => slot !== you)
  const amendment = exchange.state === 'ACTIVE'
  // The initiator is never bound to someone they have not confirmed.
  const blocked = you === 'A' && exchange.counterparty !== 'CONFIRMED'
  // Nor can someone they have not confirmed decline or answer with terms of
  // their own: they can sign, or leave.
  const signOnly = isUnconfirmedClaimant(exchange)
  // Showing payment options too, once signed: not part of what is signed.
  const [alsoShow, setAlsoShow] = useState(false)

  return (
    <section className="card" aria-labelledby="open-heading">
      <h2 id="open-heading">{amendment ? w.amendmentHeading : w.proposalHeading}</h2>
      <p className="hint">
        {fmt(w.version, { number: revision.sequence })}
        <br />
        {yours ? w.sentByYou : fmt(w.sentByOther, { name: otherName })}
        <br />
        {fmt(w.expires, { date: moment(revision.expires_at) })}
      </p>

      {revision.note && (
        <>
          <h3>{yours ? w.noteFromYou : fmt(w.noteFromOther, { name: otherName })}</h3>
          <Written>{revision.note}</Written>
        </>
      )}

      <TermsView
        terms={revision.terms}
        currency={exchange.currency}
        timezone={exchange.timezone}
        you={you}
      />
      <p className="hint fingerprint">
        {fmt(wording.terms.fingerprint, { hash: revision.content_hash })}
      </p>

      {/* What it changes, for the person asked to sign it: the author saw this while writing it. */}
      {!yours && <ProposalChanges exchange={exchange} revision={revision} />}

      <ul className="plain">
        <li>{youSigned ? w.signedByYou : w.unsignedByYou}</li>
        <li>
          {otherSigned
            ? fmt(w.signedByOther, { name: otherName })
            : fmt(w.unsignedByOther, { name: otherName })}
        </li>
      </ul>

      {yours && (
        <div className="actions">
          <Link className="button" to={paths.revise(exchange.id)}>
            {w.change}
          </Link>
          <button
            type="button"
            aria-expanded={actions.panel === 'withdraw'}
            disabled={actions.busy}
            onClick={() => actions.open('withdraw')}
          >
            {w.withdraw}
          </button>
        </div>
      )}

      {!yours && !youSigned && (
        <>
          {blocked && <p className="notice">{w.acceptBlocked}</p>}
          <div className="actions">
            {!blocked && (
              <button
                type="button"
                className="primary"
                aria-expanded={actions.panel === 'accept'}
                disabled={actions.busy}
                onClick={() => actions.open('accept')}
              >
                {w.accept}
              </button>
            )}
            {!signOnly && (
              <>
                <Link className="button" to={paths.revise(exchange.id)}>
                  {w.counter}
                </Link>
                <button
                  type="button"
                  aria-expanded={actions.panel === 'decline'}
                  disabled={actions.busy}
                  onClick={() => actions.open('decline')}
                >
                  {w.decline}
                </button>
              </>
            )}
          </div>
        </>
      )}

      {actions.panel === 'accept' && (
        <Panel title={w.signHeading}>
          <p>{w.signIntro}</p>
          <ShowWhenSigning
            terms={revision.terms}
            you={you}
            shown={Boolean(exchange.payment_options?.shown)}
            checked={alsoShow}
            onChange={setAlsoShow}
          />
          <Consent
            signLabel={w.accept}
            busy={actions.busy}
            failure={actions.failure}
            onCancel={actions.close}
            onSign={() =>
              void (async () => {
                // Acceptance names the revision: if the terms have changed
                // since this page showed them, the service refuses (DESIGN.md §6).
                const signed = await actions.run({
                  type: 'ACCEPT',
                  revision: revision.id,
                  consent: consentShown(language),
                })
                if (signed && alsoShow) {
                  await showAfterSigning(exchange.id, true)
                  await reload()
                }
              })()
            }
          />
        </Panel>
      )}

      {actions.panel === 'decline' && (
        <Panel title={w.decline}>
          <p>{amendment ? w.declineKeeps : w.declineEnds}</p>
          <Failure code={actions.failure} />
          <div className="actions">
            <button
              type="button"
              className="primary"
              disabled={actions.busy}
              onClick={() => void actions.run({ type: 'DECLINE', revision: revision.id })}
            >
              {w.confirmDecline}
            </button>
            <button type="button" disabled={actions.busy} onClick={actions.close}>
              {wording.common.cancel}
            </button>
          </div>
        </Panel>
      )}

      {actions.panel === 'withdraw' && (
        <Panel title={w.withdraw}>
          <p>{amendment ? w.withdrawKeeps : w.withdrawEnds}</p>
          <Failure code={actions.failure} />
          <div className="actions">
            <button
              type="button"
              className="primary"
              disabled={actions.busy}
              onClick={() => void actions.run({ type: 'WITHDRAW', revision: revision.id })}
            >
              {w.confirmWithdraw}
            </button>
            <button type="button" disabled={actions.busy} onClick={actions.close}>
              {wording.common.cancel}
            </button>
          </div>
        </Panel>
      )}
    </section>
  )
}
