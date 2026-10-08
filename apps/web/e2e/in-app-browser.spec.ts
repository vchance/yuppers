import { expect, test } from './support/fixtures'
import { en, fill } from './support/wording'

/*
 * A page opened from a link in another app's built-in browser, which may
 * not keep anyone signed in, says so on the sign-in form, with a way to open
 * it in Safari; Safari itself sees nothing of the sort
 * (`apps/web/src/lib/in-app-browser.ts`). The browser here is Chromium
 * sending those apps' user agents, at a phone's width.
 */

const OUTLOOK_IOS =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Outlook-iOS/749.4.prod.iphone (4.2534.0)'
const SAFARI_IOS =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1'

const PHONE = { width: 390, height: 844 }

test('in Outlook on an iPhone the sign-in form says to open the page in Safari', async ({
  browser,
}) => {
  const context = await browser.newContext({ userAgent: OUTLOOK_IOS, viewport: PHONE })
  const page = await context.newPage()
  await page.goto('/account?from=outlook')

  const note = page.getByRole('note', { name: fill(en.inAppBrowser.noteIos, { app: 'Outlook' }) })
  await expect(note).toBeVisible()
  const open = note.getByRole('link', { name: en.inAppBrowser.openSafari })
  const origin = new URL(page.url())
  // The page's own address, its query left behind, in Safari's scheme.
  await expect(open).toHaveAttribute(
    'href',
    `x-safari-${origin.protocol.slice(0, -1)}://${origin.host}/account`,
  )
  await expect(note.getByRole('button', { name: en.inAppBrowser.copy })).toBeVisible()

  // Hidden, it stays hidden for the visit.
  await note.getByRole('button', { name: en.inAppBrowser.hide }).click()
  await expect(page.getByRole('note')).toHaveCount(0)
  await expect(page.getByLabel(en.signIn.identifierLabel)).toBeFocused()
  await context.close()
})

test('in Safari on an iPhone there is no such note', async ({ browser }) => {
  const context = await browser.newContext({ userAgent: SAFARI_IOS, viewport: PHONE })
  const page = await context.newPage()
  await page.goto('/')
  await expect(page.getByLabel(en.signIn.identifierLabel)).toBeVisible()
  await expect(page.getByRole('note')).toHaveCount(0)
  await expect(page.getByText(en.inAppBrowser.openSafari)).toHaveCount(0)
  await context.close()
})
