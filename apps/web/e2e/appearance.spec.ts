import { expect, test } from './support/fixtures'
import { en } from './support/wording'

/*
 * The appearance: the device's light or dark setting unless this browser
 * chose otherwise, from the footer on any page, and applied before the
 * first paint by the one inline script the Content-Security-Policy allows
 * (the fixtures fail the test on anything it refuses).
 */

const PAGE = {
  light: 'rgb(244, 244, 239)',
  dark: 'rgb(17, 19, 39)',
}

test('follows the system, and a choice in the footer overrides it on every page from then on', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana
  const background = () => page.evaluate(() => getComputedStyle(document.documentElement).backgroundColor)

  await page.emulateMedia({ colorScheme: 'dark' })
  await page.goto('/')
  await expect(page.getByRole('heading', { name: en.signIn.title, level: 1 })).toBeVisible()
  expect(await background()).toBe(PAGE.dark)
  await page.emulateMedia({ colorScheme: 'light' })
  expect(await background()).toBe(PAGE.light)

  const footer = page.getByRole('contentinfo')
  await footer.getByRole('button', { name: en.appearance.heading }).click()
  const appearance = footer.getByRole('group', { name: en.appearance.heading })
  await appearance.getByRole('radio', { name: en.appearance.dark }).check()
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  expect(await background()).toBe(PAGE.dark)

  // Set before the app runs: on the invitation page, and on the privacy
  // policy as the service writes it, with scripts' work not yet done.
  for (const address of ['/en/i', '/privacy']) {
    await page.goto(address)
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
    expect(await background()).toBe(PAGE.dark)
  }

  await page.getByRole('contentinfo').getByRole('button', { name: en.appearance.heading }).click()
  await page
    .getByRole('contentinfo')
    .getByRole('radio', { name: en.appearance.system })
    .check()
  await expect(page.locator('html')).not.toHaveAttribute('data-theme', /.*/)
  await page.reload()
  await expect(page.locator('html')).not.toHaveAttribute('data-theme', /.*/)
  expect(await background()).toBe(PAGE.light)
})
