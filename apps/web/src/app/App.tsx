import type { Account, ErrorCode } from '@yuppers/api-client'
import {
  directionOf,
  isClientTooOld,
  languages,
  pickLanguage,
  type Language,
  type LegalDocument,
  type Wording,
} from '@yuppers/shared'
import {
  createElement,
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'

import { AccountDeleted } from '../components/AccountDeleted'
import { FooterAppearance } from '../components/Appearance'
import { LiveRegions } from '../components/LiveRegions'
import { Failure, PageHeading } from '../components/ui'
import { api, failureCode, onClientTooOld, onSignedOut, WEB_CLIENT } from '../lib/api'
import { InvitationPage } from '../screens/InvitationPage'
import {
  createI18n,
  I18nContext,
  isComplete,
  SessionContext,
  useI18n,
  useSession,
  type Session,
} from './context'
import { Link } from './Link'
import { loadedLegalPage, loadLegalPage } from './legal-page'
import { usePathname } from './router'
import { matchRoute, paths } from './routes'
import { loadWording, rememberLanguage } from './wording'

// The invitation page is the front door and has a size budget, so it is the
// only screen in the first bundle. Everything else loads when it is needed
// (DESIGN.md §13.5).
const AccountSetup = lazy(() => import('../screens/AccountSetup'))
const HomePage = lazy(() => import('../screens/HomePage'))
const AccountPage = lazy(() => import('../screens/AccountPage'))
const PaymentOptionsPage = lazy(() => import('../screens/PaymentOptionsPage'))
const DeleteAccount = lazy(() => import('../screens/DeleteAccount'))
const ExchangePage = lazy(() => import('../screens/ExchangePage'))
const RecordPage = lazy(() => import('../screens/RecordPage'))
const HelpPage = lazy(() => import('../screens/HelpPage'))
const LazyLegalPage = lazy(loadLegalPage)
// Reviewers only, and English only is acceptable for it (DESIGN.md §9).
const StaffPage = lazy(() => import('../screens/StaffPage'))

interface Props {
  initialLanguage: Language
  initialWording: Wording
}

export function App({ initialLanguage, initialWording }: Props) {
  const [account, setAccountState] = useState<Account | null>(null)
  const [ready, setReady] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [attempt, setAttempt] = useState(0)

  // The account's language once signed in, this device's before that
  // (DESIGN.md §4.2). The device takes up the language of whoever signs in,
  // so signing out leaves the screen in the language it was in.
  const [deviceChoice, setDeviceChoice] = useState(initialLanguage)
  const setAccount = useCallback((next: Account | null) => {
    setAccountState(next)
    if (!next) return
    const language = pickLanguage([next.language])
    rememberLanguage(language)
    setDeviceChoice(language)
  }, [])

  useEffect(() => {
    let cancelled = false
    api.me().then(
      (found) => {
        if (cancelled) return
        setAccount(found)
        setFailure(null)
        setReady(true)
      },
      (error: unknown) => {
        if (cancelled) return
        setFailure(failureCode(error))
        setReady(true)
      },
    )
    return () => {
      cancelled = true
    }
  }, [attempt, setAccount])

  // A session can end at any time: it expires, or is signed out elsewhere.
  useEffect(() => onSignedOut(() => setAccount(null)), [setAccount])

  // A page left open can fall behind the service. It asks at startup how old
  // a build may be, and is told again if the service refuses a change from
  // it; either way it stops offering changes it cannot make.
  const [outdated, setOutdated] = useState(false)
  useEffect(() => onClientTooOld(() => setOutdated(true)), [])
  useEffect(() => {
    let cancelled = false
    api.meta().then(
      (meta) => {
        if (!cancelled && isClientTooOld(meta.minimum_client_versions, WEB_CLIENT)) {
          setOutdated(true)
        }
      },
      () => {
        // Nothing to compare against; the service's refusals still apply.
      },
    )
    return () => {
      cancelled = true
    }
  }, [])

  // The wording, `lang`, `dir` and every format change together, once the
  // new language's wording has arrived.
  const [shown, setShown] = useState({ language: initialLanguage, wording: initialWording })
  const wanted = account ? pickLanguage([account.language]) : deviceChoice

  useEffect(() => {
    if (wanted === shown.language) return
    let cancelled = false
    loadWording(wanted).then(
      (wording) => {
        if (!cancelled) setShown({ language: wanted, wording })
      },
      () => {
        // Offline, most likely. The language already on screen stays.
      },
    )
    return () => {
      cancelled = true
    }
  }, [wanted, shown.language])

  useEffect(() => {
    document.documentElement.lang = shown.language
    document.documentElement.dir = directionOf(shown.language)
  }, [shown.language])

  const signedIn = account !== null
  const setLanguage = useCallback(
    (language: Language) => {
      rememberLanguage(language)
      setDeviceChoice(language)
      if (signedIn) {
        // The preference belongs to the account and follows it to other devices.
        api.updateMe({ language }).then(setAccount, () => {})
      }
    },
    [signedIn, setAccount],
  )

  const i18n = useMemo(
    () => createI18n(shown.language, shown.wording, setLanguage),
    [shown, setLanguage],
  )
  const session = useMemo<Session>(
    () => ({
      ready,
      failure,
      account,
      setAccount,
      retry: () => {
        setReady(false)
        setAttempt((count) => count + 1)
      },
    }),
    [ready, failure, account, setAccount],
  )

  return (
    <I18nContext value={i18n}>
      <SessionContext value={session}>
        <Shell outdated={outdated} />
      </SessionContext>
    </I18nContext>
  )
}

function Shell({ outdated }: { outdated: boolean }) {
  const { wording, language, setLanguage } = useI18n()
  const { account } = useSession()
  const pathname = usePathname()
  const route = matchRoute(pathname)
  const current = (name: string) => (route.name === name ? 'page' : undefined)

  let page: ReactNode
  if (outdated) {
    page = <Outdated />
  } else switch (route.name) {
    case 'invitation':
      page = <InvitationPage />
      break
    case 'home':
      page = (
        <Gate>
          <HomePage />
        </Gate>
      )
      break
    case 'account':
      page = (
        <Gate>
          <AccountPage />
        </Gate>
      )
      break
    case 'payments':
      page = (
        <Gate>
          <PaymentOptionsPage />
        </Gate>
      )
      break
    case 'exchange':
    case 'revise':
      page = (
        <Gate>
          <ExchangePage key={route.id} id={route.id} revising={route.name === 'revise'} />
        </Gate>
      )
      break
    case 'record':
      page = (
        <Gate>
          <RecordPage key={route.id} id={route.id} />
        </Gate>
      )
      break
    case 'help':
      // Open to anyone, signed in or not.
      page = <HelpPage key={route.topic ?? ''} topic={route.topic} />
      break
    case 'legal':
      // Open to anyone, signed in or not.
      page = <LegalRoute key={route.document} document={route.document} />
      break
    case 'staff':
    case 'staffReport':
      // For reviewers only: to anyone else the service answers "not found",
      // and so does the page. Nothing in the app links here.
      page = (
        <Gate>
          <StaffPage
            key={route.name === 'staffReport' ? route.id : ''}
            report={route.name === 'staffReport' ? route.id : null}
          />
        </Gate>
      )
      break
    default:
      page = <NotFound />
  }

  return (
    <>
      <a className="skip" href="#content">
        {wording.common.skipToContent}
      </a>
      <header className="site">
        <Link to={paths.home} className="brand">
          {wording.productName}
        </Link>
        {account && isComplete(account) && (
          <nav aria-label={wording.nav.label}>
            <Link to={paths.home} aria-current={current('home')}>
              {wording.nav.exchanges}
            </Link>
            <Link
              to={paths.account}
              aria-current={route.name === 'payments' ? 'true' : current('account')}
            >
              {wording.nav.account}
            </Link>
          </nav>
        )}
        <label className="language">
          <span className="visually-hidden">{wording.nav.language}</span>
          <select
            value={language}
            onChange={(event) => setLanguage(event.target.value as Language)}
          >
            {languages.map((info) => (
              <option key={info.code} value={info.code} lang={info.code}>
                {info.name}
              </option>
            ))}
          </select>
        </label>
      </header>
      <main id="content" tabIndex={-1}>
        <AccountDeleted />
        <Suspense fallback={<p>{wording.common.loading}</p>}>{page}</Suspense>
      </main>
      <footer className="site">
        <Link to={paths.help()} aria-current={current('help')}>
          {wording.help.link}
        </Link>
        <Link
          to={paths.legal('privacy', language)}
          aria-current={route.name === 'legal' && route.document === 'privacy' ? 'page' : undefined}
        >
          {wording.privacy.link}
        </Link>
        <Link
          to={paths.legal('terms', language)}
          aria-current={route.name === 'legal' && route.document === 'terms' ? 'page' : undefined}
        >
          {wording.termsOfUse.link}
        </Link>
        <FooterAppearance />
      </footer>
      <LiveRegions />
    </>
  )
}

/**
 * The privacy policy's or the terms' page: drawn at once if `main.tsx`
 * fetched it before the app first drew, over the same document the service
 * wrote into the page, and otherwise loaded like any other page.
 */
function LegalRoute({ document }: { document: LegalDocument }) {
  const loaded = loadedLegalPage()
  // The component the module exports, made once by it, not here.
  return loaded ? createElement(loaded, { document }) : <LazyLegalPage document={document} />
}

/** Shows a page only to a signed-in account that is ready to act; otherwise, the way to become one. */
function Gate({ children }: { children: ReactNode }) {
  const { wording } = useI18n()
  const { ready, failure, account, retry } = useSession()

  if (!ready) return <p>{wording.common.loading}</p>
  if (failure) {
    return (
      <>
        <Failure code={failure} />
        <button type="button" onClick={retry}>
          {wording.common.tryAgain}
        </button>
      </>
    )
  }
  if (!account || !isComplete(account)) {
    return (
      <>
        <AccountSetup />
        {/* An account exists from the first sign-in, before it has a name: it can be deleted from here. */}
        {account && <DeleteAccount account={account} />}
      </>
    )
  }
  return children
}

function NotFound() {
  const { wording } = useI18n()
  return (
    <>
      <PageHeading>{wording.common.notFoundTitle}</PageHeading>
      <p>{wording.common.notFoundBody}</p>
      <p>
        <Link to={paths.home}>{wording.common.goHome}</Link>
      </p>
    </>
  )
}

/**
 * This build is older than the service accepts changes from. On the web the
 * update is a reload, so that is what is offered, and nothing else is: a page
 * that cannot act must not look as if it can.
 */
function Outdated() {
  const { wording } = useI18n()
  return (
    <>
      <PageHeading>{wording.errors.CLIENT_TOO_OLD}</PageHeading>
      <p>{wording.service.outdatedWeb}</p>
      <div className="actions">
        <button type="button" className="primary" onClick={() => window.location.reload()}>
          {wording.service.reload}
        </button>
      </div>
    </>
  )
}
