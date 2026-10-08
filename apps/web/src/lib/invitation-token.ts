import { invitationToken } from '../app/routes'

/*
 * The token from an invitation link, between opening the link and claiming
 * the invitation.
 *
 * It arrives in the fragment and is taken out of the address bar at once, so
 * it is not left on screen, in a screenshot or in a copied address
 * (DESIGN.md §13.5). It is kept for this tab only, because a phone often
 * reloads the page while its owner is away reading the one-time code, and a
 * link that stopped working at that moment would strand them. It is removed
 * as soon as the invitation is claimed or turns out to be dead. This is the
 * invitation token, which only lets its holder read one proposal; the session
 * is a cookie the page cannot read and is never stored here.
 */

const KEY = 'yuppers.invitation'

function stored(): string | null {
  try {
    return window.sessionStorage.getItem(KEY)
  } catch {
    return null
  }
}

/** The token for the invitation page being shown, moving it out of the address bar. */
export function takeInvitationToken(): string | null {
  const fromLink = invitationToken(window.location.hash)
  if (!fromLink) return stored()
  try {
    window.sessionStorage.setItem(KEY, fromLink)
  } catch {
    // Without storage the token lasts until the page is reloaded.
  }
  window.history.replaceState(window.history.state, '', window.location.pathname)
  return fromLink
}

/**
 * The token this tab holds for the invitation page, without taking anything
 * out of the address bar: for opening the same invitation link in another
 * browser (`in-app-browser.ts`).
 */
export function heldInvitationToken(): string | null {
  return invitationToken(window.location.hash) ?? stored()
}

/**
 * Forgets `token`, unless another link has taken its place in this tab since:
 * an answer about the old one can arrive after the new one was opened.
 */
export function forgetInvitationToken(token: string): void {
  try {
    if (window.sessionStorage.getItem(KEY) === token) window.sessionStorage.removeItem(KEY)
  } catch {
    // Nothing was stored.
  }
}
