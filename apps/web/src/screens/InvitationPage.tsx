import type { ErrorCode } from '@yuppers/api-client'
import { labelText } from '@yuppers/shared'
import { lazy, Suspense, useEffect, useRef, useState } from 'react'

import { isComplete, useI18n, useSession } from '../app/context'
import { Link } from '../app/Link'
import { navigate } from '../app/router'
import { invitationToken, paths } from '../app/routes'
import { Mark } from '../components/Mark'
import { TermsView } from '../components/TermsView'
import { ErrorNote, Failure, PageHeading, Written } from '../components/ui'
import { useAnnouncement } from '../lib/announce'
import { api, failureCode, type InvitationPreview } from '../lib/api'
import { forgetInvitationToken, takeInvitationToken } from '../lib/invitation-token'
import { SignIn } from './SignIn'

// Only someone signed in and reading the proposal needs these.
const AccountSetup = lazy(() => import('./AccountSetup'))
const InvitationReport = lazy(() =>
  import('./InvitationReport').then((module) => ({ default: module.InvitationReport })),
)
// Only someone signed in with another address than the one invited.
const InvitationAddress = lazy(() =>
  import('../components/InvitationAddress').then((module) => ({
    default: module.InvitationAddress,
  })),
)

/**
 * Where an invitation link lands. The proposal is read signed in: someone
 * signed out is shown only that a yup is waiting and the way to sign in,
 * the same for every link, live or dead. The service answers nothing about a
 * link to someone signed out, so a blocked person has nothing to compare
 * with the dead link they are shown signed in (DESIGN.md §9). Responding
 * means claiming the invitation, which takes the invited party's place in
 * the exchange (DESIGN.md §8). Reading claims nothing.
 *
 * A second link pasted into a tab already showing this page changes only
 * the fragment, so the browser does not load the page again. The token is
 * taken again then, and a different one shows its own proposal from the
 * start, with nothing kept from the one before.
 */
export function InvitationPage() {
  const { account } = useSession()
  // Taking the token also removes it from the address bar.
  const [token, setToken] = useState(takeInvitationToken)
  useEffect(() => {
    const taken = () => {
      // A fragment that is not a token, or no fragment at all, leaves the
      // page as it is: taking the token is what emptied it.
      if (!invitationToken(window.location.hash)) return
      setToken(takeInvitationToken())
    }
    window.addEventListener('hashchange', taken)
    return () => window.removeEventListener('hashchange', taken)
  }, [])
  // Whoever signs in, or out, starts from the top: nothing one person was
  // shown stays on the page for the next.
  return <Invitation key={`${token ?? ''} ${account?.id ?? ''}`} token={token} />
}

function Invitation({ token }: { token: string | null }) {
  const { wording } = useI18n()
  const { account, ready } = useSession()
  const w = wording.invitation

  if (!token) {
    return (
      <>
        <PageHeading>{w.missingTitle}</PageHeading>
        <p>{w.missing}</p>
      </>
    )
  }
  if (!ready) return <p>{wording.common.loading}</p>
  if (!account) {
    // The same page for every link: nothing here depends on the token.
    return (
      <>
        <PageHeading>{w.signedOutTitle}</PageHeading>
        <p>{w.signInToRead}</p>
        <section aria-labelledby="invitation-sign-in">
          <h2 id="invitation-sign-in">{wording.signIn.title}</h2>
          <SignIn />
        </section>
      </>
    )
  }
  return <Proposal token={token} />
}

/** The proposal behind a link, for the account signed in. */
function Proposal({ token }: { token: string }) {
  const { wording, fmt, moment } = useI18n()
  const { account, setAccount } = useSession()
  const w = wording.invitation

  const [preview, setPreview] = useState<InvitationPreview | null>(null)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [spent, setSpent] = useState(false)
  const [responding, setResponding] = useState(false)
  const claiming = useRef(false)

  useEffect(() => {
    let cancelled = false
    api.previewInvitation(token).then(
      (found) => {
        if (!cancelled) setPreview(found)
      },
      (error: unknown) => {
        if (cancelled) return
        // A session that has ended signs the page out by itself.
        const code = failureCode(error)
        if (code === 'INVITATION_UNAVAILABLE') setSpent(true)
        else setFailure(code)
      },
    )
    return () => {
      cancelled = true
    }
  }, [token])

  const able = account !== null && isComplete(account)
  useAnnouncement(responding && able ? w.opening : null)

  // A link that no longer shows its proposal has usually been used, and
  // people come back to the message it arrived in. The service is asked
  // whether the place it gave is already this account's, which takes
  // nothing whatever the answer: the person who used the link is taken to
  // the exchange, and anyone else gets the same refusal as before. Only the
  // button below ever claims.
  useEffect(() => {
    if (!spent) return
    if (!able) {
      forgetInvitationToken(token)
      return
    }
    if (claiming.current) return
    claiming.current = true
    api.invitationAlreadyYours(token).then(
      (exchange) => {
        forgetInvitationToken(token)
        navigate(paths.exchange(exchange.id), { replace: true })
      },
      (error: unknown) => {
        forgetInvitationToken(token)
        claiming.current = false
        setFailure(failureCode(error))
      },
    )
  }, [spent, able, token])

  // Once the person has asked to respond and has an account that can, claim.
  useEffect(() => {
    if (!responding || !able || claiming.current) return
    claiming.current = true
    api.claimInvitation(token).then(
      (exchange) => {
        forgetInvitationToken(token)
        navigate(paths.exchange(exchange.id), { replace: true })
      },
      (error: unknown) => {
        const code = failureCode(error)
        if (code === 'INVITATION_UNAVAILABLE') forgetInvitationToken(token)
        claiming.current = false
        setResponding(false)
        setFailure(code)
      },
    )
  }, [responding, able, token])

  const sender = preview?.revision.terms.party_a_name ?? ''

  // A link that cannot be read is refused at the top of an otherwise empty
  // page. A claim is refused next to the button that asked for it, which is
  // below the whole proposal.
  const refused: ErrorCode | null = failure ?? (spent && !able ? 'INVITATION_UNAVAILABLE' : null)
  const refusal = refused && (
    <>
      {/* The only refusal a claim gives for this reason is opening one's own link. */}
      {refused === 'ACTION_NOT_ALLOWED' ? (
        <ErrorNote>{w.ownInvitation}</ErrorNote>
      ) : (
        <Failure code={refused} />
      )}
      {refused === 'INVITATION_NOT_FOR_YOU' && (
        <p>
          <button
            type="button"
            onClick={() => {
              api.signOut().then(
                () => setAccount(null),
                () => setAccount(null),
              )
            }}
          >
            {wording.nav.signOut}
          </button>
        </p>
      )}
      <p>
        <Link to={paths.home}>{wording.common.goHome}</Link>
      </p>
    </>
  )

  return (
    <>
      <PageHeading>{w.title}</PageHeading>

      {!preview && refusal}
      {!preview && !refused && <p>{wording.common.loading}</p>}

      {preview && (
        <>
          {/* The yup at a glance, the one tilted callout on the page: who it
              is from in their colour and who it is for in the reader's. It
              says nothing the terms below do not, so it is hidden from
              assistive technology, and holds none of the terms. */}
          <div className="yup-card" aria-hidden="true">
            <span className="yup-card-them">
              <bdi>{labelText(sender)}</bdi>
            </span>
            <span className="yup-card-you">
              <bdi>{labelText(preview.revision.terms.party_b_name) || wording.party.other}</bdi>
            </span>
            <span className="yup-card-seam">
              <Mark />
            </span>
          </div>
          {/* An invitation that names nobody can be opened by whoever holds
              the link, so its sender has to confirm them before they can do
              more than sign (DESIGN.md §8). */}
          <p>
            {fmt(preview.bound ? w.introSignedIn : wording.claimant.invitationIntroSignedIn, {
              name: sender,
            })}
          </p>
          <p>{w.notBinding}</p>

          <section
            className="card"
            aria-label={fmt(wording.home.reference, { code: preview.display_code })}
          >
            <p className="hint">{fmt(wording.home.reference, { code: preview.display_code })}</p>
            {preview.revision.note && (
              <>
                <h2>{fmt(w.noteHeading, { name: sender })}</h2>
                <Written>{preview.revision.note}</Written>
              </>
            )}
            <TermsView
              terms={preview.revision.terms}
              currency={preview.currency}
              timezone={preview.timezone}
              you={null}
              level={2}
            />
            <p className="hint">{fmt(w.expires, { date: moment(preview.revision.expires_at) })}</p>
          </section>

          {preview.bound && !preview.sent_to && <p>{w.boundSignedIn}</p>}
          {refusal}

          {/* Sent to an address this account does not have: add it first. */}
          {preview.sent_to && (
            <Suspense fallback={<p>{wording.common.loading}</p>}>
              <InvitationAddress
                token={token}
                sentTo={preview.sent_to}
                onOpened={(exchange) => {
                  forgetInvitationToken(token)
                  navigate(paths.exchange(exchange.id), { replace: true })
                }}
                onSignOut={() => {
                  api.signOut().then(
                    () => setAccount(null),
                    () => setAccount(null),
                  )
                }}
              />
            </Suspense>
          )}

          {responding && able && <p>{w.opening}</p>}
          {responding && !able && (
            <Suspense fallback={<p>{wording.common.loading}</p>}>
              <AccountSetup headingLevel="h2" />
            </Suspense>
          )}
          {!responding && !preview.sent_to && (
            <div className="actions">
              <button
                type="button"
                className="primary"
                onClick={() => {
                  setFailure(null)
                  setResponding(true)
                }}
              >
                {account && able ? fmt(w.respondAs, { name: account.display_name }) : w.respondNew}
              </button>
            </div>
          )}
          <Suspense fallback={null}>
            <InvitationReport token={token} />
          </Suspense>
        </>
      )}
    </>
  )
}
