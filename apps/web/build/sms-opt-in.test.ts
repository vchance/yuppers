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

/** The code texts as the service sends them, word for word (`backend/src/notifications/sms.rs`). */
const CODE_TEXTS: Record<string, { signIn: string; verifyNumber: string }> = {
  en: {
    signIn: 'Yuppers.app: 123456 is your sign-in code. Do not share it with anyone.',
    verifyNumber: 'Yuppers.app: 123456 is your code to confirm this phone number. Do not share it with anyone.',
  },
  es: {
    signIn: 'Yuppers.app: 123456 es tu código para entrar. No se lo des a nadie.',
    verifyNumber: 'Yuppers.app: 123456: código para confirmar tu número. No lo compartas.',
  },
}

describe('the page on how people opt in to texts', () => {
  test('one per language: /sms-opt-in in the default, /{language}/sms-opt-in in the others', () => {
    expect(pages.map((page) => [page.path, page.fileName])).toEqual([
      ['/sms-opt-in', 'sms-opt-in/index.html'],
      ['/es/sms-opt-in', 'es/sms-opt-in/index.html'],
    ])
  })

  test.each(['en', 'es'])('in %s shows the seven steps, each with what it says word for word', (language) => {
    const page = parse(language)
    const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
    const product = read(`${language}.json`)
    expect(page.documentElement.lang).toBe(language)
    expect(page.title).toBe(`${wording.title} · Yuppers`)
    const root = page.getElementById('root')!
    expect(root.querySelector('h1')?.textContent).toBe(wording.title)
    expect(root.textContent).toContain(wording.note)

    const website = root.querySelector('#website')!.parentElement!
    expect(website.querySelector('h2')?.textContent).toBe(wording.websiteHeading)
    const steps = [...website.querySelectorAll('section.step')]
    expect(steps.map((step) => step.querySelector('h3')?.id)).toEqual([
      'sign-in',
      'box-ticked',
      'code-sent',
      'text-updates',
      'confirmation',
      'confirmation-text',
      'help-and-stop',
    ])
    const quoted = (index: number) =>
      [...steps[index].querySelectorAll('q, .sms p')].map((element) => element.textContent)
    const links = (index: number) =>
      [...steps[index].querySelectorAll('q a')].map((link) => link.getAttribute('href'))
    const addresses = ['https://yuppers.app/terms', 'https://yuppers.app/privacy']

    // (a) The sign-in form with a number entered: the box beside it, with
    // the words the terms quote, and "Send code" waiting for it.
    expect(quoted(0)).toEqual(
      expect.arrayContaining([
        product.signIn.identifierLabel,
        product.smsCode.signIn,
        product.smsCode.tickToSend,
        product.signIn.sendCode,
        product.privacy.smsLink,
        product.privacy.policy,
        product.termsOfUse.document,
      ]),
    )
    expect(quoted(0)).not.toContain('Message and data rates may apply. Reply STOP to opt out.')
    expect(links(0)).toEqual(addresses)
    // (b) The box ticked, and "Send code".
    expect(quoted(1)).toEqual([product.smsCode.signIn, product.signIn.sendCode])
    expect(links(1)).toEqual(addresses)
    // (c) After asking for a code, and the code's text.
    expect(quoted(2)).toEqual(
      expect.arrayContaining([
        fill(product.signIn.codeSent, { identifier: SAMPLE_PHONE }),
        CODE_TEXTS[language].signIn,
      ]),
    )
    expect(quoted(2)).toContain(fill(product.sms.signIn, { code: '123456' }))
    // (d) The box beside the consent wording, its addresses links, and the
    // box beside a number being added.
    expect(quoted(3)).toContain(product.smsUpdates.consent)
    expect(quoted(3)).toContain(product.smsCode.verifyNumber)
    // The text carrying the code that checks the number, last.
    expect(quoted(3).at(-1)).toBe(CODE_TEXTS[language].verifyNumber)
    expect(quoted(3).at(-1)).toBe(fill(product.sms.verifyNumber, { code: '123456' }))
    expect(steps[3].textContent).toContain(wording.verifyCodeText)
    expect(links(3)).toEqual([...addresses, ...addresses])
    // (e) The confirmation on screen.
    expect(quoted(4)).toEqual([
      fill(product.smsUpdates.on, { phone: `+1 •••-•••-${SAMPLE_PHONE.slice(-4)}` }),
    ])
    // (f) The opt-in confirmation text.
    expect(quoted(5)).toEqual([product.sms.optInConfirmation])
    // (g) HELP and STOP, and an update as it arrives.
    expect(quoted(6)).toEqual([
      'HELP',
      wording.replies.help,
      'STOP',
      wording.replies.stop,
      fill(product.sms.update, { link: SAMPLE_LINK }),
    ])
  })

  test.each(['en', 'es'])('in %s opens with a contents list of its two parts', (language) => {
    const page = parse(language)
    const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
    const contents = page.querySelector(`nav[aria-label="${wording.contentsLabel}"]`)!
    const links = [...contents.querySelectorAll('a')]
    expect(links.map((link) => [link.getAttribute('href'), link.textContent])).toEqual([
      ['#website', wording.websiteHeading],
      ['#mobile-app', wording.mobileHeading],
    ])
    // Before either part, and each link leads to its part's heading.
    const article = page.querySelector('article')!
    const first = article.querySelector('section.part')!
    expect(contents.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    for (const link of links) {
      const target = page.getElementById(link.getAttribute('href')!.slice(1))!
      expect(target.tagName).toBe('H2')
      expect(target.parentElement?.matches('section.part')).toBe(true)
    }
  })

  test.each(['en', 'es'])(
    'in %s shows the five steps in the mobile app, each with its caption and what it says word for word',
    (language) => {
      const page = parse(language)
      const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
      const product = read(`${language}.json`)
      const heading = page.getElementById('mobile-app')!
      expect(heading.textContent).toBe(wording.mobileHeading)
      const part = heading.parentElement!
      // The app's release state, and that its consent and confirmation are the website's.
      expect(part.textContent).toContain(wording.mobileRelease)
      expect(wording.mobileRelease).toMatch(/App Store/)
      expect(wording.mobileRelease).toMatch(/Google Play/)
      const same = fill(wording.mobileSame, {
        confirmationText: wording.confirmationTextLink,
        replies: wording.repliesLink,
      })
      expect(part.textContent).toContain(same)
      expect([...part.querySelectorAll('p > a')].map((link) => link.getAttribute('href'))).toEqual([
        '#confirmation-text',
        '#help-and-stop',
      ])

      const steps = [...part.querySelectorAll('section.step')]
      expect(steps.map((step) => step.querySelector('h3')?.id)).toEqual([
        'mobile-sign-in',
        'mobile-box-ticked',
        'mobile-code-sent',
        'mobile-text-updates',
        'mobile-confirmation',
      ])
      ;(Object.keys(SCREENSHOTS) as Screen[]).forEach((screen, index) => {
        const step = steps[index]
        expect(step.querySelector('h3')?.textContent).toBe(wording.mobileSteps[screen].title)
        expect(step.querySelector('h3 + p')?.textContent).toBe(wording.mobileSteps[screen].caption)
        const image = step.querySelector('img')!
        expect(image.getAttribute('src')).toBe(screenshotPath(screen, language, 'en', 'mobile'))
        expect(image.getAttribute('alt')).toBe(wording.mobileSteps[screen].alt)
      })
      const quoted = (index: number) =>
        [...steps[index].querySelectorAll('q')].map((element) => element.textContent)

      const addresses = ['https://yuppers.app/terms', 'https://yuppers.app/privacy']
      const links = (index: number) =>
        [...steps[index].querySelectorAll('q a')].map((link) => link.getAttribute('href'))
      // (m1) The app's sign-in screen with a number entered: the box, the
      // button waiting for it, and the links.
      expect(quoted(0)).toEqual(
        expect.arrayContaining([
          product.signIn.title,
          product.signIn.identifierLabel,
          product.smsCode.signIn,
          product.smsCode.tickToSend,
          product.signIn.sendCode,
          product.privacy.smsLink,
          product.privacy.policy,
          product.termsOfUse.document,
        ]),
      )
      expect(links(0)).toEqual(addresses)
      // (m2) The box ticked.
      expect(quoted(1)).toEqual([product.smsCode.signIn, product.signIn.sendCode])
      // (m3) After asking for a code.
      expect(quoted(2)).toEqual(
        expect.arrayContaining([fill(product.signIn.codeSent, { identifier: SAMPLE_PHONE })]),
      )
      // (m4) The same consent wording as the website's, its addresses links.
      expect(quoted(3)).toEqual(
        expect.arrayContaining([
          product.smsUpdates.consent,
          product.smsUpdates.save,
          product.smsCode.verifyNumber,
        ]),
      )
      expect(links(3)).toEqual([...addresses, ...addresses])
      // (m5) The same confirmation as the website's.
      const on = fill(product.smsUpdates.on, { phone: `+1 •••-•••-${SAMPLE_PHONE.slice(-4)}` })
      expect(quoted(4)).toEqual([on])
      const website = page.getElementById('confirmation')!.parentElement!
      expect([...website.querySelectorAll('q')].map((element) => element.textContent)).toEqual([on])
    },
  )

  test.each(['en', 'es'])('in %s shows a picture of each screen, from this origin', (language) => {
    const images = [...parse(language).querySelectorAll('section.step img')]
    expect(images.map((image) => image.getAttribute('src'))).toEqual(
      (['web', 'mobile'] as const).flatMap((surface) =>
        (Object.keys(SCREENSHOTS) as Screen[]).map((screen) =>
          screenshotPath(screen, language, 'en', surface),
        ),
      ),
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

  test('keeps the support address in the HELP reply readable without scripts', () => {
    for (const language of ['en', 'es']) {
      const page = html(language)
      const start = page.indexOf('<!--email_off-->')
      const end = page.indexOf('<!--/email_off-->')
      const at = page.indexOf('support@yuppers.app')
      expect(start).toBeGreaterThan(0)
      expect(at > start && at < end).toBe(true)
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
    // And the link to give for the mobile app's screens is this page's part on it.
    expect(guide).toContain('https://yuppers.app/sms-opt-in#mobile-app')
    expect(read('sms-opt-in/es.json').replies).toEqual(replies)
    // The samples of the code texts are the service's, in both languages.
    for (const language of ['en', 'es']) {
      const { sms } = read(`${language}.json`)
      for (const text of [sms.signIn, sms.deleteAccount, sms.verifyNumber]) {
        expect(guide).toContain(`"${fill(text, { code: '123456' })}"`)
      }
    }
  })
})
