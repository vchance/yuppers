import {
  boundToLabel,
  invitationLink,
  inviteeKind,
  phoneAsTyped,
  phoneOffered,
  shareAddresses,
  type InvitationChoice,
  type SignInChannels,
} from '@yuppers/shared'
import { useEffect, useId, useMemo, useRef, useState } from 'react'

import { useI18n } from '../app/context'
import { ErrorNote, Field, Notice } from './ui'

/**
 * Who an invitation is for (DESIGN.md §8). Naming them is what is expected:
 * only an account verified with that email address or phone number can use
 * the link, and needs no confirming, so they can respond in full as soon as
 * they sign in. The field says why, that we do not contact them, and that
 * they must sign in with exactly what is typed. A link for anyone is offered
 * after it as a deliberate choice, which says what it costs once chosen.
 *
 * An email address only, until the service says it texts codes (`channels`),
 * as signing in asks: a phone number nobody can sign in with would make the
 * link useless.
 */
export function InvitationFor({
  choice,
  onChange,
  channels,
  error,
  id: fixedId,
}: {
  choice: InvitationChoice
  onChange(choice: InvitationChoice): void
  channels: SignInChannels | null
  /** What is wrong with it, said under it once the person has tried to go on. */
  error?: string | null
  /** The field's id, for the keyboard to be taken to it. */
  id?: string
}) {
  const { wording } = useI18n()
  const w = wording.invitationLink
  const generated = useId()
  const id = fixedId ?? generated
  const anyoneNote = useRef<HTMLDivElement>(null)
  // Set when the person switches, so the keyboard follows them, and not
  // when the form is first shown.
  const switched = useRef(false)

  useEffect(() => {
    if (!switched.current) return
    switched.current = false
    if (choice.anyone) anyoneNote.current?.focus()
    else document.getElementById(id)?.focus()
  }, [choice.anyone, id])

  function choose(anyone: boolean) {
    switched.current = true
    onChange({ ...choice, anyone })
  }

  if (choice.anyone) {
    return (
      <div className="invitation-for">
        {/* Focused when chosen, so what it costs is read out at once. */}
        <div className="notice notice-warning" ref={anyoneNote} tabIndex={-1}>
          <p>{w.forAnyoneText}</p>
        </div>
        <div className="actions">
          <button type="button" className="link" onClick={() => choose(false)}>
            {w.forNamed}
          </button>
        </div>
      </div>
    )
  }

  return (
    <div className="invitation-for">
      <p>{w.forIntro}</p>
      <p>{w.forNoContact}</p>
      <Field label={boundToLabel(w, channels)} hint={w.forHint} error={error} id={id} required>
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
            value={choice.to}
            onChange={(event) => onChange({ ...choice, to: event.target.value })}
            // A US number is written the American way on leaving the field.
            onBlur={() => {
              const shown = phoneAsTyped(choice.to)
              if (shown !== choice.to) onChange({ ...choice, to: shown })
            }}
          />
        )}
      </Field>
      <div className="actions">
        <button type="button" className="link" onClick={() => choose(true)}>
          {w.forAnyone}
        </button>
      </div>
    </div>
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

interface ShareProps {
  token: string
  /** Who the invitation was made for, as typed, when it names someone. */
  boundTo: string | null
  /**
   * Called when the person opens any way to pass the link on. The service
   * is told, so the exchange's page and the list can say the link was sent,
   * or that it was not.
   */
  onShared?: () => void
}

/**
 * The ways to send the link (DESIGN.md §8), led by the one that reaches the
 * person it was made for. Yuppers never sends it; one of these has to be
 * opened by the sender, so each is a real button or link, and the first of
 * them is the primary action:
 *
 *   - named by phone number: a text message to that number, with WhatsApp
 *     and copying the link beside it;
 *   - named by email address: an email to that address, with copying beside it;
 *   - for anyone: the device's share sheet where the browser has one, and
 *     otherwise copying the link, with a text message, WhatsApp and an email
 *     to nobody in particular beside it;
 *
 * and, for someone in the same room, the link as a QR code. Each message is
 * started by this device, with the whole link inside it (`shareAddresses`);
 * the token reaches no server of ours.
 */
export function ShareActions({ token, boundTo, onShared }: ShareProps) {
  const { wording, language, fmt } = useI18n()
  const w = wording.invitationLink
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null)
  const [qr, setQr] = useState(false)
  const link = invitationLink(window.location.origin, language, token)
  const data = { title: wording.linkPreview.title, text: w.shareText, url: link }
  const [native] = useState(() => nativeShare(data))
  const addresses = shareAddresses({
    message: fmt(w.shareMessage, { link }),
    subject: wording.linkPreview.title,
    boundTo,
  })
  const kind = inviteeKind(boundTo)

  useEffect(() => {
    // Nothing to do if it fails now; showing the code asks again.
    loadQr().catch(() => {})
  }, [])

  const shared = () => onShared?.()

  async function copy() {
    shared()
    try {
      await navigator.clipboard.writeText(link)
      setCopied('yes')
    } catch {
      setCopied('failed')
    }
  }

  function share() {
    shared()
    // Dismissing the share sheet rejects; there is nothing to do about it.
    navigator.share(data).catch(() => {})
  }

  function toggleQr() {
    // Showing the code is a way of passing the link on; hiding it is not.
    if (!qr) shared()
    setQr((shown) => !shown)
  }

  const cls = (primary: boolean) => (primary ? 'button primary' : 'button')
  const sms = (label: string, primary = false) => (
    <a className={cls(primary)} href={addresses.sms} onClick={shared}>
      {label}
    </a>
  )
  const email = (label: string, primary = false) => (
    <a className={cls(primary)} href={addresses.email} onClick={shared}>
      {label}
    </a>
  )
  const whatsApp = (label: string) => (
    <a
      className="button"
      href={addresses.whatsApp}
      target="_blank"
      rel="noopener noreferrer"
      onClick={shared}
    >
      {label}
      <span className="visually-hidden"> {wording.help.newTab}</span>
    </a>
  )
  const copyButton = (primary = false) => (
    <button type="button" className={primary ? 'primary' : undefined} onClick={() => void copy()}>
      {w.copy}
    </button>
  )

  return (
    <div className="share">
      <div className="actions share-actions">
        {kind === 'phone' && (
          <>
            {sms(w.sendText, true)}
            {whatsApp(w.sendWhatsApp)}
            {copyButton()}
          </>
        )}
        {kind === 'email' && (
          <>
            {email(w.sendEmail, true)}
            {copyButton()}
          </>
        )}
        {kind === 'anyone' && (
          <>
            {native ? (
              <button type="button" className="primary" onClick={share}>
                {w.share}
              </button>
            ) : null}
            {copyButton(!native)}
            {sms(w.shareSms)}
            {whatsApp(w.shareWhatsApp)}
            {email(w.shareEmail)}
          </>
        )}
        <button type="button" aria-expanded={qr} onClick={toggleQr}>
          {qr ? w.hideQr : w.shareQr}
        </button>
      </div>
      {qr && <QrPanel link={link} />}
      {copied === 'yes' && <Notice>{w.copied}</Notice>}
      {copied === 'failed' && <ErrorNote>{w.copyFailed}</ErrorNote>}
    </div>
  )
}

/**
 * The invitation link, shown once: only its hash is kept by the service, so
 * it cannot be shown again. The initiator sends it through a channel of
 * their own; the platform never does (DESIGN.md §8), and says so where the
 * link appears. Under it, the ways to send it (`ShareActions`).
 *
 * The link is in the sender's language, so the preview a messaging app builds
 * for it is too. What is shared alongside it is fixed wording with no name,
 * term or amount in it.
 */
export function InvitationLink({ token, boundTo, onShared }: ShareProps) {
  const { wording, language } = useI18n()
  const w = wording.invitationLink
  const link = invitationLink(window.location.origin, language, token)

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
      <ShareActions token={token} boundTo={boundTo} onShared={onShared} />
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
