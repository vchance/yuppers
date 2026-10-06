// @vitest-environment jsdom
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import axe from 'axe-core'
import { describe, expect, test } from 'vitest'

import { renderEntryPage } from './entry-pages.ts'
import {
  SAMPLE_LINK,
  SAMPLE_PHONE,
  SCREENSHOTS,
  asSmsOptInPage,
  readSmsOptInPages,
  screenshotPath,
  type Screen,
  type SmsOptInWording,
} from './sms-opt-in.ts'

/*
 * The page on how people opt in to texts, as the build writes it: every
 * step, its picture, and the exact wording of each screen and message, read
 * without scripts, with nothing inline.
 */

const wordingDirectory = join(import.meta.dirname, '../../../packages/shared/wording')
const publicDirectory = join(import.meta.dirname, '../public')
const repoRoot = join(import.meta.dirname, '../../..')
const template = readFileSync(join(import.meta.dirname, '../index.html'), 'utf8')
const read = (name: string) =>
  JSON.parse(readFileSync(`${wordingDirectory}/${name}`, 'utf8')) as Record<string, any>

const pages = readSmsOptInPages(wordingDirectory)
const html = (language: string) => {
  const page = pages.find((candidate) => candidate.lang === language)!
  return asSmsOptInPage(renderEntryPage(template, page), page)
}
const parse = (language: string) => new DOMParser().parseFromString(html(language), 'text/html')
const fill = (message: string, values: Record<string, string>) =>
  message.replace(/\{(\w+)\}/g, (marker, name: string) => values[name] ?? marker)

describe('the page on how people opt in to texts', () => {
  test('one per language: /sms-opt-in in the default, /{language}/sms-opt-in in the others', () => {
    expect(pages.map((page) => [page.path, page.fileName])).toEqual([
      ['/sms-opt-in', 'sms-opt-in/index.html'],
      ['/es/sms-opt-in', 'es/sms-opt-in/index.html'],
    ])
  })

  test.each(['en', 'es'])('in %s shows the six steps, each with what it says word for word', (language) => {
    const page = parse(language)
    const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
    const product = read(`${language}.json`)
    expect(page.documentElement.lang).toBe(language)
    expect(page.title).toBe(`${wording.title} · Yuppers`)
    const root = page.getElementById('root')!
    expect(root.querySelector('h1')?.textContent).toBe(wording.title)
    expect(root.textContent).toContain(wording.note)

    const steps = [...root.querySelectorAll('section.step')]
    expect(steps.map((step) => step.querySelector('h2')?.id)).toEqual([
      'sign-in',
      'code-sent',
      'text-updates',
      'confirmation',
      'confirmation-text',
      'help-and-stop',
    ])
    const quoted = (index: number) =>
      [...steps[index].querySelectorAll('q, .sms p')].map((element) => element.textContent)

    // (a) The sign-in form, with what texts cost and how to stop them.
    expect(quoted(0)).toEqual(
      expect.arrayContaining([
        product.signIn.identifierLabel,
        product.signIn.sendCode,
        `${product.privacy.sms} ${product.privacy.smsLink}`,
        product.privacy.policy,
        product.termsOfUse.document,
      ]),
    )
    // (b) After asking for a code, and the code's text.
    expect(quoted(1)).toEqual(
      expect.arrayContaining([
        fill(product.signIn.codeSent, { identifier: SAMPLE_PHONE }),
        fill(product.sms.signIn, { code: '123456', productName: 'Yuppers' }),
      ]),
    )
    // (c) The box beside the consent wording, its addresses links.
    expect(quoted(2)).toContain(product.smsUpdates.consent)
    expect(
      [...steps[2].querySelectorAll('q a')].map((link) => link.getAttribute('href')),
    ).toEqual(['https://yuppers.app/terms', 'https://yuppers.app/privacy'])
    // (d) The confirmation on screen.
    expect(quoted(3)).toEqual([
      fill(product.smsUpdates.on, { phone: `+1 •••-•••-${SAMPLE_PHONE.slice(-4)}` }),
    ])
    // (e) The opt-in confirmation text.
    expect(quoted(4)).toEqual([product.sms.optInConfirmation])
    // (f) HELP and STOP, and an update as it arrives.
    expect(quoted(5)).toEqual([
      'HELP',
      wording.replies.help,
      'STOP',
      wording.replies.stop,
      fill(product.sms.update, { link: SAMPLE_LINK }),
    ])
  })

  test.each(['en', 'es'])('in %s shows a picture of each screen, from this origin', (language) => {
    const images = [...parse(language).querySelectorAll('section.step img')]
    expect(images.map((image) => image.getAttribute('src'))).toEqual(
      (Object.keys(SCREENSHOTS) as Screen[]).map((screen) => screenshotPath(screen, language, 'en')),
    )
    for (const image of images) {
      const src = image.getAttribute('src')!
      expect(src.startsWith('/sms-opt-in/'), src).toBe(true)
      expect(existsSync(join(publicDirectory, src)), src).toBe(true)
      expect(image.getAttribute('alt')?.length, src).toBeGreaterThan(20)
    }
  })

  test('links to the terms and the privacy policy on texts, and they link back', () => {
    const page = parse('en')
    const links = [...page.querySelectorAll('main a')].map((link) => link.getAttribute('href'))
    expect(links).toEqual(
      expect.arrayContaining(['/terms#text-messages', '/privacy#text-messages', '/es/sms-opt-in']),
    )
    for (const document of ['terms', 'privacy']) {
      for (const language of ['en', 'es']) {
        const text = JSON.stringify(read(`${document}/${language}.json`).sections['text-messages'])
        expect(text, `${document} ${language}`).toContain('{smsOptIn}')
      }
    }
  })

  test('runs no script, has nothing inline, and keeps the app’s stylesheet with its own', () => {
    for (const language of ['en', 'es']) {
      const page = html(language)
      expect(page).not.toMatch(/<script|modulepreload/)
      expect(page).not.toMatch(/<style|\sstyle=|\son[a-z]+=/i)
      expect(page).not.toMatch(/\{\{\w+\}\}/)
      expect(page).toContain('<link rel="stylesheet" href="/sms-opt-in/page.css">')
    }
    expect(existsSync(join(publicDirectory, 'sms-opt-in/page.css'))).toBe(true)
  })

  test.each(['en', 'es'])('in %s passes axe', async (language) => {
    const page = parse(language)
    window.document.documentElement.lang = page.documentElement.lang
    window.document.title = page.title
    window.document.head.innerHTML = page.head.innerHTML
    window.document.body.innerHTML = page.body.innerHTML
    const results = await axe.run(window.document, {
      runOnly: {
        type: 'tag',
        values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'],
      },
      rules: { 'color-contrast': { enabled: false } },
    })
    expect(
      results.violations.map(
        (violation) => `${violation.id}: ${violation.nodes.map((node) => node.html).join(' ')}`,
      ),
    ).toEqual([])
  })

  test('the HELP and STOP replies are the ones the deployment guide has Twilio send', () => {
    const guide = readFileSync(join(repoRoot, 'docs/deploy-render.md'), 'utf8')
    const { replies } = read('sms-opt-in/en.json') as SmsOptInWording
    expect(guide).toContain(replies.help)
    expect(guide).toContain(replies.stop)
    expect(read('sms-opt-in/es.json').replies).toEqual(replies)
  })
})
