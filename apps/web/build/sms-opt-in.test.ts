// @vitest-environment jsdom
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import axe from 'axe-core'
import { describe, expect, test } from 'vitest'

import { renderEntryPage } from './entry-pages.ts'
import {
  FORMS,
  PART_ANCHORS,
  SAMPLE_LINK,
  SAMPLE_PHONE,
  SAMPLE_PHONE_TO_ADD,
  SCREENSHOTS,
  asSmsOptInPage,
  formAnchor,
  readSmsOptInPages,
  screenshotPath,
  stepAnchor,
  type Form,
  type Screen,
  type SmsOptInWording,
  type Surface,
} from './sms-opt-in.ts'

/*
 * The page on how people opt in to texts, as the build writes it: every
 * form on the website and in the app, each step with its picture, and the
 * exact wording of each screen and message, read without scripts, with
 * nothing inline.
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
const masked = (phone: string) => `+1 •••-•••-${phone.slice(-4)}`

const SURFACES: Surface[] = ['web', 'mobile']
const FORM_NAMES = Object.keys(FORMS) as Form[]

/**
 * The code texts as the backend sends them, word for word: the cases of its
 * own test, `the_code_messages_read_as_written_and_are_sent_as_expected` in
 * `backend/src/notifications/sms.rs`, by language and reason.
 */
const CODE_TEXTS = (() => {
  const source = readFileSync(join(repoRoot, 'backend/src/notifications/sms.rs'), 'utf8')
  const found: Record<string, Record<string, string>> = {}
  for (const [, language, reason, text] of source.matchAll(
    /\(\s*"(\w+)",\s*CodePurpose::(\w+),\s*"([^"]+)",/g,
  )) {
    found[language] = { ...found[language], [reason]: text }
  }
  return found
})()

/** A form's section, under its heading's anchor. */
function formSection(page: Document, form: Form, surface: Surface): Element {
  const heading = page.getElementById(formAnchor(form, surface))
  expect(heading?.tagName, formAnchor(form, surface)).toBe('H3')
  return heading!.parentElement!
}

/** What a step quotes: the screen's words, then the texts that follow it. */
function quotedIn(step: Element): string[] {
  return [...step.querySelectorAll('q, .sms p')].map((element) => element.textContent ?? '')
}

/** What the step with a picture of `screen` quotes, in a part. */
function quotedAt(page: Document, screen: Screen, surface: Surface): string[] {
  return quotedIn(page.getElementById(stepAnchor(screen, surface))!.parentElement!)
}

describe('the page on how people opt in to texts', () => {
  test('one per language: /sms-opt-in in the default, /{language}/sms-opt-in in the others', () => {
    expect(pages.map((page) => [page.path, page.fileName])).toEqual([
      ['/sms-opt-in', 'sms-opt-in/index.html'],
      ['/es/sms-opt-in', 'es/sms-opt-in/index.html'],
    ])
  })

  test('the backend’s code texts are read, all three reasons in both languages', () => {
    for (const language of ['en', 'es']) {
      expect(Object.keys(CODE_TEXTS[language] ?? {}).sort(), language).toEqual([
        'DeleteAccount',
        'SignIn',
        'VerifyNumber',
      ])
    }
  })

  test.each(['en', 'es'])(
    'in %s lists all four forms and HELP and STOP on the website and in the app, each under its anchor',
    (language) => {
      const page = parse(language)
      const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
      expect(page.documentElement.lang).toBe(language)
      expect(page.title).toBe(`${wording.title} · Yuppers`)
      const root = page.getElementById('root')!
      expect(root.querySelector('h1')?.textContent).toBe(wording.title)
      expect(root.textContent).toContain(wording.note)

      for (const surface of SURFACES) {
        const part = page.getElementById(PART_ANCHORS[surface])!
        expect(part.tagName).toBe('H2')
        expect(part.textContent).toBe(surface === 'web' ? wording.websiteHeading : wording.mobileHeading)
        const section = part.parentElement!
        expect(section.matches('section.part')).toBe(true)
        // The forms, in order, each with its steps under their own anchors.
        expect([...section.querySelectorAll(':scope > section.form > h3')].map((h) => h.id)).toEqual(
          FORM_NAMES.map((form) => formAnchor(form, surface)),
        )
        for (const form of FORM_NAMES) {
          const each = formSection(page, form, surface)
          expect(each.querySelector('h3')?.textContent).toBe(wording.forms[form].heading)
          expect(each.querySelector('h3 + p')?.textContent).toBe(wording.forms[form].intro)
          const steps = (surface === 'web' ? wording.steps : wording.mobileSteps) as SmsOptInWording['steps']
          expect([...each.querySelectorAll('section.step > h4')].map((h) => [h.id, h.textContent])).toEqual(
            FORMS[form].map((step) => [stepAnchor(step, surface), steps[step].title]),
          )
          for (const step of FORMS[form]) {
            const section = page.getElementById(stepAnchor(step, surface))!.parentElement!
            expect(section.querySelector('h4 + p')?.textContent).toBe(steps[step].caption)
            if (step === 'confirmationText') continue
            const image = section.querySelector('img')!
            expect(image.getAttribute('src')).toBe(screenshotPath(step, language, 'en', surface))
            expect(image.getAttribute('alt')).toBe(steps[step].alt)
          }
        }
      }
    },
  )

  test.each(['en', 'es'])('in %s keeps the anchors it had, and has one for each form', (language) => {
    const page = parse(language)
    const anchors = [
      'website',
      'mobile-app',
      'sign-in',
      'box-ticked',
      'code-sent',
      'text-updates',
      'confirmation',
      'confirmation-text',
      'help-and-stop',
      'mobile-sign-in',
      'mobile-box-ticked',
      'mobile-code-sent',
      'mobile-text-updates',
      'mobile-confirmation',
      'website-sign-in',
      'website-confirm-number',
      'website-delete-account',
      'website-agreement-updates',
      'website-help-and-stop',
      'mobile-app-sign-in',
      'mobile-app-confirm-number',
      'mobile-app-delete-account',
      'mobile-app-agreement-updates',
      'mobile-app-help-and-stop',
      'confirm-number',
      'confirm-number-ticked',
      'confirm-number-code-sent',
      'delete-account',
      'delete-account-ticked',
      'delete-account-code-sent',
      'mobile-confirm-number',
      'mobile-confirm-number-ticked',
      'mobile-confirm-number-code-sent',
      'mobile-delete-account',
      'mobile-delete-account-ticked',
      'mobile-delete-account-code-sent',
      'mobile-confirmation-text',
    ]
    for (const anchor of anchors) expect(page.getElementById(anchor), anchor).not.toBeNull()
    // No anchor is used twice.
    const ids = [...page.querySelectorAll('[id]')].map((element) => element.id)
    expect(new Set(ids).size).toBe(ids.length)
    // HELP and STOP on the website keeps the address it had as a step.
    expect(page.getElementById('help-and-stop')?.querySelector('h3')?.id).toBe('website-help-and-stop')
  })

  test.each(['en', 'es'])(
    'in %s quotes each consent label word for word from the wording file, beside a box unticked and then ticked',
    (language) => {
      const page = parse(language)
      const product = read(`${language}.json`)
      const labels: [Exclude<Form, 'replies'>, string][] = [
        ['signIn', product.smsCode.signIn],
        ['confirmNumber', product.smsCode.verifyNumber],
        ['deleteAccount', product.smsCode.deleteAccount],
        ['agreementUpdates', product.smsUpdates.consent],
      ]
      const addresses = ['https://yuppers.app/terms', 'https://yuppers.app/privacy']
      for (const surface of SURFACES) {
        for (const [form, label] of labels) {
          const [unticked, second] = FORMS[form]
          const first = page.getElementById(stepAnchor(unticked, surface))!.parentElement!
          expect(quotedIn(first), `${surface} ${form}`).toContain(label)
          // Its two addresses are links that read as the addresses.
          const links = [...first.querySelectorAll('q a')].map((link) => link.getAttribute('href'))
          expect(links, `${surface} ${form}`).toEqual(addresses)
          // Each form that texts a code says to tick the box first, and then
          // shows it ticked beside the same words.
          if (form !== 'agreementUpdates') {
            expect(quotedIn(first)).toContain(product.smsCode.tickToSend)
            const ticked = page.getElementById(stepAnchor(second, surface))!.parentElement!
            expect(quotedIn(ticked)[0], `${surface} ${form}`).toBe(label)
          }
        }
        // The buttons each form's box holds back.
        const ticked = (step: Screen) =>
          quotedIn(page.getElementById(stepAnchor(step, surface))!.parentElement!)
        expect(ticked('boxTicked')).toEqual([product.smsCode.signIn, product.signIn.sendCode])
        expect(ticked('confirmNumberTicked')).toEqual([
          product.smsCode.verifyNumber,
          product.smsUpdates.sendCode,
        ])
        expect(ticked('deleteAccountTicked')).toEqual([
          product.smsCode.deleteAccount,
          product.deletion.sendCode,
        ])
      }
    },
  )

  test.each(['en', 'es'])(
    'in %s quotes each code text, the confirmation and the replies word for word as the service sends them',
    (language) => {
      const page = parse(language)
      const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
      const product = read(`${language}.json`)
      const codes = CODE_TEXTS[language]
      for (const surface of SURFACES) {
        const after = (step: Screen) => quotedAt(page, step, surface)
        // Each "code sent" screen: what it says, then the text that carries
        // the code, the backend's own, from the service's wording.
        const signedIn = after('codeSent')
        expect(signedIn).toContain(fill(product.signIn.codeSent, { identifier: SAMPLE_PHONE }))
        expect(signedIn.at(-1)).toBe(codes.SignIn)
        expect(signedIn.at(-1)).toBe(fill(product.sms.signIn, { code: '123456' }))

        const confirmed = after('confirmNumberCodeSent')
        expect(confirmed).toContain(
          fill(product.smsUpdates.codeSent, { phone: masked(SAMPLE_PHONE_TO_ADD) }),
        )
        expect(confirmed).toContain(product.smsUpdates.codeLabel)
        expect(confirmed.at(-1)).toBe(codes.VerifyNumber)
        expect(confirmed.at(-1)).toBe(fill(product.sms.verifyNumber, { code: '123456' }))
        const confirmStep = page.getElementById(stepAnchor('confirmNumberCodeSent', surface))!
        expect(confirmStep.parentElement!.textContent).toContain(wording.verifyCodeText)

        const deleting = after('deleteAccount')
        expect(deleting).toContain(fill(product.deletion.codeIntro, { identifier: SAMPLE_PHONE }))
        const deleted = after('deleteAccountCodeSent')
        expect(deleted).toContain(fill(product.deletion.codeSent, { identifier: SAMPLE_PHONE }))
        expect(deleted.at(-1)).toBe(codes.DeleteAccount)
        expect(deleted.at(-1)).toBe(fill(product.sms.deleteAccount, { code: '123456' }))
        const deleteStep = page.getElementById(stepAnchor('deleteAccountCodeSent', surface))!
        expect(deleteStep.parentElement!.textContent).toContain(wording.deleteCodeText)

        // Agreement updates: the confirmation on screen, then its text.
        expect(after('confirmation')).toEqual([
          fill(product.smsUpdates.on, { phone: masked(SAMPLE_PHONE) }),
        ])
        const confirmationText = page.getElementById(stepAnchor('confirmationText', surface))!
        expect(quotedIn(confirmationText.parentElement!)).toEqual([product.sms.optInConfirmation])

        // HELP and STOP, and an update as it arrives.
        expect(quotedIn(formSection(page, 'replies', surface))).toEqual([
          'HELP',
          wording.replies.help,
          'STOP',
          wording.replies.stop,
          fill(product.sms.update, { link: SAMPLE_LINK }),
        ])
      }
      // The sign-in form as it first appears, with its links.
      for (const surface of SURFACES) {
        expect(quotedAt(page, 'signIn', surface)).toEqual(
          expect.arrayContaining([
            product.signIn.identifierLabel,
            product.signIn.sendCode,
            product.privacy.smsLink,
            product.privacy.policy,
            product.termsOfUse.document,
          ]),
        )
      }
    },
  )

  test.each(['en', 'es'])('in %s opens with a contents list of every form in both parts', (language) => {
    const page = parse(language)
    const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
    const contents = page.querySelector(`nav[aria-label="${wording.contentsLabel}"]`)!
    const links = [...contents.querySelectorAll('a')]
    expect(links.map((link) => [link.getAttribute('href'), link.textContent])).toEqual(
      SURFACES.flatMap((surface) => [
        [`#${PART_ANCHORS[surface]}`, surface === 'web' ? wording.websiteHeading : wording.mobileHeading],
        ...FORM_NAMES.map((form) => [`#${formAnchor(form, surface)}`, wording.forms[form].heading]),
      ]),
    )
    // Before either part, and each link leads to a heading.
    const article = page.querySelector('article')!
    const first = article.querySelector('section.part')!
    expect(contents.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    for (const link of links) {
      const target = page.getElementById(link.getAttribute('href')!.slice(1))!
      expect(['H2', 'H3']).toContain(target.tagName)
    }
  })

  test.each(['en', 'es'])("in %s says the app's release state, and that its words are the website's", (language) => {
    const page = parse(language)
    const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
    const part = page.getElementById('mobile-app')!.parentElement!
    expect(part.textContent).toContain(wording.mobileIntro)
    expect(part.textContent).toContain(wording.mobileRelease)
    expect(wording.mobileRelease).toMatch(/App Store/)
    expect(wording.mobileRelease).toMatch(/Google Play/)
    expect(part.textContent).toContain(wording.mobileSame)
  })

  test.each(['en', 'es'])('in %s shows a picture of each screen, from this origin', (language) => {
    const images = [...parse(language).querySelectorAll('section.step img')]
    expect(images.map((image) => image.getAttribute('src'))).toEqual(
      SURFACES.flatMap((surface) =>
        FORM_NAMES.flatMap((form) =>
          (FORMS[form] as readonly string[])
            .filter((step): step is Screen => step in SCREENSHOTS)
            .map((screen) => screenshotPath(screen, language, 'en', surface)),
        ),
      ),
    )
    expect(images).toHaveLength(2 * Object.keys(SCREENSHOTS).length)
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

  test('keeps the support address in the HELP replies readable without scripts', () => {
    for (const language of ['en', 'es']) {
      const page = html(language)
      const start = page.indexOf('<!--email_off-->')
      const end = page.indexOf('<!--/email_off-->')
      expect(start).toBeGreaterThan(0)
      let at = page.indexOf('support@yuppers.app')
      expect(at).toBeGreaterThan(0)
      while (at !== -1) {
        expect(at > start && at < end).toBe(true)
        at = page.indexOf('support@yuppers.app', at + 1)
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
    // The links to give for each form, on the website and in the app, are
    // this page's anchors.
    expect(guide).toContain('https://yuppers.app/sms-opt-in#mobile-app')
    for (const surface of SURFACES) {
      for (const form of FORM_NAMES) {
        expect(guide).toContain(`https://yuppers.app/sms-opt-in#${formAnchor(form, surface)}`)
      }
    }
    // The samples of the code texts are the service's, in both languages.
    for (const language of ['en', 'es']) {
      const { sms } = read(`${language}.json`)
      for (const text of [sms.signIn, sms.deleteAccount, sms.verifyNumber]) {
        expect(guide).toContain(`"${fill(text, { code: '123456' })}"`)
      }
    }
  })
})
