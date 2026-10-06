import { useSyncExternalStore } from 'react'

/*
 * Routing on the History API. The app has half a dozen addresses, matched in
 * `routes.ts`, and the invitation page has a size budget (DESIGN.md §13.5),
 * so this and `Link` are the whole router and no library is loaded for it.
 */

const listeners = new Set<() => void>()
let navigated = false

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  window.addEventListener('popstate', listener)
  return () => {
    listeners.delete(listener)
    window.removeEventListener('popstate', listener)
  }
}

export function navigate(to: string, options: { replace?: boolean } = {}): void {
  if (options.replace) window.history.replaceState(null, '', to)
  else window.history.pushState(null, '', to)
  navigated = true
  window.scrollTo(0, 0)
  for (const listener of listeners) listener()
}

/**
 * Changes the address without moving to another page or scrolling: the
 * same page under the address that names what it now shows, such as the
 * privacy policy's in the language it is read in. The fragment is kept.
 */
export function replaceAddress(to: string): void {
  window.history.replaceState(window.history.state, '', `${to}${window.location.hash}`)
  for (const listener of listeners) listener()
}

/**
 * Whether the person has moved between pages since the app loaded. A page
 * reached that way takes the focus to its heading; the first page leaves the
 * focus where the browser put it.
 */
export function hasNavigated(): boolean {
  return navigated
}

export function usePathname(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname)
}
