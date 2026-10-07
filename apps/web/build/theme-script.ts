import { createHash } from 'node:crypto'

/*
 * The appearance this device chose (System, Light or Dark, `src/lib/theme.ts`)
 * is applied before the page first paints, so a page never flashes in the
 * wrong one: this script is the first thing in every page's `<head>`
 * (`renderEntryPage`). With nothing stored, or anything else stored, it does
 * nothing, and `color-scheme: light dark` follows the system.
 *
 * It is the one inline script on any page. The Content-Security-Policy
 * allows it by its hash and nothing else inline
 * (`CONTENT_SECURITY_POLICY_VALUE` in `backend/src/http/mod.rs`), so a change
 * here changes the hash there too; `theme-script.test.ts` checks they agree.
 */
export const THEME_SCRIPT =
  "try{var t=localStorage.getItem('yuppers.theme');if(t==='light'||t==='dark')document.documentElement.setAttribute('data-theme',t)}catch(e){}"

/** The script's CSP source expression, `'sha256-…'`. */
export function themeScriptHash(): string {
  return `'sha256-${createHash('sha256').update(THEME_SCRIPT, 'utf8').digest('base64')}'`
}

/** Puts the script first in a page's `<head>`. */
export function withThemeScript(html: string): string {
  if (!html.includes('<head>')) throw new Error('the page has no <head> for the theme script')
  return html.replace('<head>', `<head>\n    <script>${THEME_SCRIPT}</script>`)
}
