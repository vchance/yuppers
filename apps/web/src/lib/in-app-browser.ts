/*
 * Whether the page is running inside another app's built-in browser, such
 * as the one an email or social app opens a link in, and how to get the page
 * out of it and into the person's own browser.
 *
 * Why it matters: such a browser keeps its cookies apart from Safari's or
 * Chrome's, and often forgets them, so someone who opens every link from
 * their email is asked for a new sign-in code again and again. A note on the
 * sign-in form says so (`components/InAppBrowserNote.tsx`).
 *
 * Detection is best effort, from the user agent alone, and errs towards
 * saying nothing: the note must never appear in plain Safari or Chrome.
 *
 * What can be told:
 *   - Apps that add a token of their own to their browser's user agent:
 *     Facebook (`FBAN/`, `FBAV/`, `FB_IAB`), Messenger, Instagram
 *     (`Instagram 3…`), LinkedIn (`LinkedInApp`), Snapchat (`Snapchat/`),
 *     TikTok (`musical_ly`, `BytedanceWebview`, `TikTok`), X (`Twitter`,
 *     `TwitterAndroid`), LINE (`Line/`), the Google app (`GSA/`), and Outlook
 *     where its token (`Outlook-iOS/`, `Outlook-Android/`) is present. These
 *     are the tokens the apps are widely reported to send and that the
 *     maintained detection libraries match (for example `inapp-spy`); none
 *     is documented by the apps, and any can change with an update.
 *   - On iOS, a web view an app runs itself (`WKWebView`) that adds nothing:
 *     its user agent is Safari's without the `Safari/…` token at the end.
 *     Every browser on iOS, Chrome (`CriOS`), Firefox (`FxiOS`), Edge
 *     (`EdgiOS`) and the rest, keeps that token, so its absence is the
 *     usual sign of an in-app browser; the app is then not named. A page
 *     added to the home screen has no such token either, and is left out
 *     (`standalone`).
 *   - On Android, an app's `WebView`, which marks itself `; wv)`.
 *
 * What cannot be told:
 *   - On iOS, `SFSafariViewController`, the Safari sheet many apps open links
 *     in (Gmail and X among them, depending on settings), sends exactly
 *     Safari's user agent. It keeps cookies apart from Safari too, but
 *     nothing in the request says it is not Safari, so nothing is shown.
 *     Whether Outlook for iOS opens links in such a sheet or in a web view of
 *     its own has not been confirmed here; in a web view without its token
 *     it is caught by the rule above, unnamed.
 *   - On Android, Chrome Custom Tabs, which Gmail and many others open links
 *     in, are Chrome itself, with Chrome's cookies: there is nothing to say.
 *   - Any app that sends a browser's user agent unchanged.
 */

export type InAppPlatform = 'ios' | 'android' | 'other'

export interface InAppBrowser {
  /** The app, by its own name, where its user agent says; `null` where it does not. */
  app: string | null
  platform: InAppPlatform
  /** The iOS major version, where the user agent gives it. */
  iosVersion: number | null
}

/** Apps that name themselves, in the order they are checked: Messenger before Facebook. */
const NAMED: [RegExp, string][] = [
  [/\bFBAN\/Messenger|\bFB_IAB\/MESSENGER|\bMessengerForiOS|\bOrca-Android/i, 'Messenger'],
  [/\bFBAN\/|\bFBAV\/|\bFB_IAB\b|\bFBIOS\b|\bMetaIAB\b/, 'Facebook'],
  [/\bInstagram\b/, 'Instagram'],
  [/\bLinkedInApp\b/, 'LinkedIn'],
  [/\bSnapchat\//, 'Snapchat'],
  [/\bmusical_ly|\bBytedanceWebview\b|\bTikTok\b/, 'TikTok'],
  [/\bTwitter(Android)?\b/, 'X'],
  [/\bLine\/\d/, 'LINE'],
  [/\bOutlook-(iOS|Android)\b/i, 'Outlook'],
  [/\bGSA\/\d/, 'Google'],
]

/** Browsers proper, which never get the note, whatever else their user agent says. */
const BROWSERS = /\b(CriOS|FxiOS|EdgiOS|OPiOS|OPT|YaBrowser|DuckDuckGo|Ddg)\//

function platformOf(userAgent: string): InAppPlatform {
  if (/\b(iPhone|iPad|iPod)\b/.test(userAgent)) return 'ios'
  if (/\bAndroid\b/.test(userAgent)) return 'android'
  return 'other'
}

function iosVersionOf(userAgent: string): number | null {
  const found = /\bOS (\d+)[_.]\d+(?:[_.]\d+)? like Mac OS X/.exec(userAgent)
  return found ? Number(found[1]) : null
}

/**
 * The in-app browser this user agent belongs to, or `null` for a browser
 * proper and for anything that cannot be told apart from one. `standalone`
 * is whether the page runs as an app added to the home screen.
 */
export function detectInAppBrowser(userAgent: string, standalone = false): InAppBrowser | null {
  if (standalone) return null
  const platform = platformOf(userAgent)
  const iosVersion = platform === 'ios' ? iosVersionOf(userAgent) : null
  const named = NAMED.find(([pattern]) => pattern.test(userAgent))
  if (named) return { app: named[1], platform, iosVersion }
  if (BROWSERS.test(userAgent)) return null
  if (
    platform === 'ios' &&
    /AppleWebKit\//.test(userAgent) &&
    /\bMobile\//.test(userAgent) &&
    !/\bSafari\//.test(userAgent)
  ) {
    return { app: null, platform, iosVersion }
  }
  if (platform === 'android' && /; wv\)/.test(userAgent)) return { app: null, platform, iosVersion }
  return null
}

/** Whether the page runs as an app added to the home screen. */
export function runsStandalone(): boolean {
  const legacy = (window.navigator as Navigator & { standalone?: boolean }).standalone === true
  return legacy || window.matchMedia?.('(display-mode: standalone)').matches === true
}

/**
 * A link that opens `address` (`https:`, or `http:` in development) in the person's own
 * browser, where the platform has one; `null` where the only way is the
 * app's own menu.
 *
 *   - iOS 17 and later: `x-safari-https://…`, the scheme Safari registers
 *     for itself (with `x-safari-http`), opens the address in Safari whatever the default browser.
 *     Apple does not document it; it is reported working from iOS 17 on and
 *     not on 16, so it is offered from 17.
 *   - Android: an intent link, `intent://host/path#Intent;scheme=https;end`,
 *     asks the system to open the address with whatever handles `https:`,
 *     the default browser. Android takes the intent's own part after the
 *     last `#` (`Intent.parseUri`), so an address with a fragment keeps it.
 *     It works where the app's web view passes intent links to the system,
 *     as Chrome-based ones generally do; where it does not, nothing happens,
 *     and the note's other way, the app's menu or the copied link, remains.
 */
export function openElsewhereLink(browser: InAppBrowser, address: string): string | null {
  const url = new URL(address)
  // `https`, or `http` on a development machine.
  const scheme = url.protocol.slice(0, -1)
  if (scheme !== 'https' && scheme !== 'http') return null
  const rest = url.host + url.pathname + url.search + url.hash
  if (browser.platform === 'ios' && (browser.iosVersion ?? 0) >= 17) {
    return `x-safari-${scheme}://${rest}`
  }
  if (browser.platform === 'android') return `intent://${rest}#Intent;scheme=${scheme};end`
  return null
}

/**
 * The address to open elsewhere: this page's, without its query, which no
 * page here needs and which must never carry a code or token out of the
 * app, and without its fragment. On an invitation page the invitation's own
 * token, which the page took out of the address bar into this tab's storage
 * (`invitation-token.ts`), is put back as the fragment: the page cannot work
 * without it, and it is exactly the link that was opened.
 */
export function addressToOpen(location: Location, invitationToken: string | null): string {
  const base = location.origin + location.pathname
  return invitationToken ? `${base}#${invitationToken}` : base
}
