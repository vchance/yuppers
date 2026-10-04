import {
  boundToLabel,
  invitationLink,
  phoneOffered,
  shareAddresses,
  type SignInChannels,
} from '@yuppers/shared'
import { useEffect, useMemo, useRef, useState } from 'react'

import { useI18n } from '../app/context'
import { ErrorNote, Field, Notice } from './ui'
import { Panel } from './Panel'

/**
 * Naming who an invitation is for, so that only an account verified with
 * that email address or phone number can use the link (DESIGN.md §8). An
 * email address only, until the service says it texts codes (`channels`),
 * as signing in asks: a phone number nobody can sign in with would make the
 * link useless.
 */
export function InvitationFor({
  value,
  onChange,
  channels,
  error,
  id,
}: {
  value: string
  onChange(value: string): void
  channels: SignInChannels | null
  /** What is wrong with it, said under it once the person has tried to go on. */
  error?: string | null
  id?: string
}) {
  const { wording } = useI18n()
  return (
    <Field
      label={boundToLabel(wording.invitationLink, channels)}
      hint={wording.invitationLink.forHint}
      error={error}
      id={id}
    >
      {(control) => (
        <input
          {...control}
          // A text field where a phone number may be typed, which a browser
          // would otherwise take for a malformed email address.
          type={phoneOffered(channels) ? 'text' : 'email'}
          inputMode="email"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          value={value}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    </Field>
  )
}

type QrModule = typeof import('../lib/qr')
let qrModule: Promise<QrModule> | null = null

/**
 * The QR encoder's module, fetched once. It is asked for as soon as a link
 * is shown, never by the page that first loads, so the invitation page's
 * size is untouched and the code can still be drawn if the connection drops
 * while the link is on screen.
 */
function loadQr(): Promise<QrModule> {
  qrModule ??= import('../lib/qr').catch((error: unknown) => {
    // Asked for again next time, in case the connection is back.
    qrModule = null
    throw error
  })
  return qrModule
}

/** Whether this browser has a share sheet of its own that takes the link. */
function nativeShare(data: ShareData): boolean {
  if (typeof navigator.share !== 'function') return false
  return typeof navigator.canShare !== 'function' || navigator.canShare(data)
}

/**
 * The invitation link, shown once: only its hash is kept by the service, so
 * it cannot be shown again. The initiator sends it through a channel of
 * their own; the platform never does (DESIGN.md §8), and says so where the
 * link appears.
 *
 * "Share link" always does something. Where the browser has a share sheet,
 * as on a phone, it opens it, and copying the link and its QR code are
 * beside it. Where there is none, as in most desktop browsers, it opens a
 * panel of ways to start a message: copy the link, an email (to whom the
 * invitation is for, if that is an email address), a text message, WhatsApp,
 * or a QR code for someone in the room to scan. Each is opened by this
 * device; the token reaches no server of ours (`shareAddresses`).
 *
 * The link is in the sender's language, so the preview a messaging app builds
 * for it is too. What is shared alongside it is fixed wording with no name,
 * term or amount in it.
 */
export function InvitationLink({ token, boundTo }: { token: string; boundTo: string | null }) {
  const { wording, language, fmt } = useI18n()
  const w = wording.invitationLink
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null)
  const [menu, setMenu] = useState(false)
  const [qr, setQr] = useState(false)
  const shareButton = useRef<HTMLButtonElement>(null)
  const link = invitationLink(window.location.origin, language, token)
  const data = { title: wording.linkPreview.title, text: w.shareText, url: link }
  const [native] = useState(() => nativeShare(data))
  const addresses = shareAddresses({
    message: fmt(w.shareMessage, { link }),
    subject: wording.linkPreview.title,
    boundTo,
  })

  useEffect(() => {
    // Nothing to do if it fails now; showing the code asks again.
    loadQr().catch(() => {})
  }, [])

  async function copy() {
    try {
      await navigator.clipboard.writeText(link)
      setCopied('yes')
    } catch {
      setCopied('failed')
    }
  }

  function share() {
    if (!native) {
      // Pressed again, it closes what it opened.
      if (menu) close()
      else setMenu(true)
      return
    }
    // Dismissing the share sheet rejects; there is nothing to do about it.
    navigator.share(data).catch(() => {})
  }

  function close() {
    setMenu(false)
    setQr(false)
    shareButton.current?.focus()
  }

  const qrToggle = (
    <button type="button" aria-expanded={qr} onClick={() => setQr((shown) => !shown)}>
      {qr ? w.hideQr : w.shareQr}
    </button>
  )

  return (
    <div className="invitation-link">
      <p>{w.intro}</p>
      <Field label={w.linkLabel} hint={w.shownOnce}>
        {(control) => (
          <input
            {...control}
            type="text"
            readOnly
            dir="ltr"
            value={link}
            onFocus={(event) => event.target.select()}
          />
        )}
      </Field>
      <div className="actions">
        <button
          type="button"
          className="primary"
          ref={shareButton}
          aria-expanded={native ? undefined : menu}
          onClick={share}
        >
          {w.share}
        </button>
        {native && (
          <>
            <button type="button" onClick={() => void copy()}>
              {w.copy}
            </button>
            {qrToggle}
          </>
        )}
      </div>
      {menu && (
        <Panel title={w.share}>
          <ul className="share-options">
            <li>
              <button type="button" onClick={() => void copy()}>
                {w.copy}
              </button>
            </li>
            <li>
              <a className="button" href={addresses.email}>
                {w.shareEmail}
              </a>
            </li>
            <li>
              <a className="button" href={addresses.sms}>
                {w.shareSms}
              </a>
            </li>
            <li>
              <a
                className="button"
                href={addresses.whatsApp}
                target="_blank"
                rel="noopener noreferrer"
              >
                {w.shareWhatsApp}
                <span className="visually-hidden"> {wording.help.newTab}</span>
              </a>
            </li>
            <li>{qrToggle}</li>
          </ul>
          {qr && <QrPanel link={link} />}
          <div className="actions">
            <button type="button" onClick={close}>
              {w.closeShare}
            </button>
          </div>
        </Panel>
      )}
      {native && qr && <QrPanel link={link} />}
      {copied === 'yes' && <Notice>{w.copied}</Notice>}
      {copied === 'failed' && <ErrorNote>{w.copyFailed}</ErrorNote>}
    </div>
  )
}

/**
 * The link as a QR code, for someone in the same room to scan with their
 * phone. Drawn on the device as an inline SVG: no image is fetched or made
 * from a `data:` address, so it works offline and asks nothing of the
 * Content-Security-Policy. Dark on light whatever the theme, as cameras read
 * a light-on-dark code badly.
 */
function QrPanel({ link }: { link: string }) {
  const { wording } = useI18n()
  const w = wording.invitationLink
  const [loaded, setLoaded] = useState<QrModule | 'failed' | null>(null)

  useEffect(() => {
    let cancelled = false
    loadQr().then(
      (found) => {
        if (!cancelled) setLoaded(found)
      },
      () => {
        if (!cancelled) setLoaded('failed')
      },
    )
    return () => {
      cancelled = true
    }
  }, [])

  const drawing = useMemo(
    () => (loaded && loaded !== 'failed' ? loaded.drawQr(link) : null),
    [loaded, link],
  )

  if (loaded === 'failed') return <ErrorNote>{w.qrFailed}</ErrorNote>
  if (!drawing) return <p>{wording.common.loading}</p>
  return (
    <figure className="qr">
      <svg
        className="qr-code"
        role="img"
        aria-label={w.qrLabel}
        viewBox={`0 0 ${drawing.size} ${drawing.size}`}
        shapeRendering="crispEdges"
      >
        <rect width={drawing.size} height={drawing.size} fill="#ffffff" />
        <path d={drawing.path} fill="#000000" />
      </svg>
      <figcaption className="hint">{w.qrHint}</figcaption>
    </figure>
  )
}
