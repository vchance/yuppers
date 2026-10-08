import { useId, useRef, useState } from 'react'

import { useI18n } from '../app/context'
import { matchRoute } from '../app/routes'
import {
  addressToOpen,
  detectInAppBrowser,
  openElsewhereLink,
  runsStandalone,
  type InAppBrowser,
} from '../lib/in-app-browser'
import { heldInvitationToken } from '../lib/invitation-token'
import { ErrorNote, Notice } from './ui'

// Hidden until the page is loaded again: kept in memory only, so hiding the
// note adds nothing to what the site stores on the device (the privacy
// policy lists all of that).
let hidden = false

function detected(): InAppBrowser | null {
  try {
    return detectInAppBrowser(window.navigator.userAgent, runsStandalone())
  } catch {
    return null
  }
}

/**
 * On the sign-in form, for a page open in another app's built-in browser
 * (`lib/in-app-browser.ts`): that browser may not keep the person signed in,
 * so it says so, and offers the way out: a link that opens the page in
 * Safari or the default browser where the platform has one, the app's own
 * menu, and the link to copy. Shown on both steps, the address and the code,
 * until hidden; nothing at all in a browser proper.
 */
export function InAppBrowserNote() {
  const { wording, fmt } = useI18n()
  const w = wording.inAppBrowser
  const [browser] = useState(detected)
  const [shown, setShown] = useState(!hidden)
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null)
  const note = useRef<HTMLDivElement>(null)
  const textId = useId()

  if (!browser || !shown) return null

  // An invitation page needs its token, which is how its link came; no
  // other page carries one.
  const invitation = matchRoute(window.location.pathname).name === 'invitation'
  const address = addressToOpen(window.location, invitation ? heldInvitationToken() : null)
  const open = openElsewhereLink(browser, address)
  const app = browser.app ?? w.someApp
  const text = fmt(browser.platform === 'ios' ? w.noteIos : w.noteOther, { app })

  async function copy() {
    try {
      await window.navigator.clipboard.writeText(address)
      setCopied('yes')
    } catch {
      setCopied('failed')
    }
  }

  function hide() {
    hidden = true
    // The focus goes on to the form the note stood at the top of.
    const form = note.current?.closest('form')
    setShown(false)
    form?.querySelector<HTMLInputElement>('input')?.focus()
  }

  return (
    <div className="notice in-app-note" role="note" aria-labelledby={textId} ref={note}>
      <p id={textId}>{text}</p>
      <div className="actions">
        {open && (
          <a className="button primary" href={open}>
            {browser.platform === 'ios' ? w.openSafari : w.openBrowser}
          </a>
        )}
        <button type="button" onClick={() => void copy()}>
          {w.copy}
        </button>
        <button type="button" className="link" onClick={hide}>
          {w.hide}
        </button>
      </div>
      <p className="hint">{w.menu}</p>
      {copied === 'yes' && <Notice>{w.copied}</Notice>}
      {copied === 'failed' && <ErrorNote>{w.copyFailed}</ErrorNote>}
    </div>
  )
}
