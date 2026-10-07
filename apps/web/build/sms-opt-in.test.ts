// @vitest-environment jsdom
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import axe from 'axe-core'
import { describe, expect, test } from 'vitest'

import { renderEntryPage } from './entry-pages.ts'
import {
  CODES_ANCHOR,
  CODE_FORMS,
  FORMS,
  PART_ANCHORS,
  SAMPLE_LINK,
  SAMPLE_PHONE,
  SAMPLE_PHONE_TO_ADD,
  SCREENSHOTS,
  UPDATE_FORMS,
  VERIFY_DELETION_SAMPLE,
  VERIFY_SAMPLE,
  asSmsOptInPage,
  codesAnchor,
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
 * The page on how people opt in to texts, as the build writes it: the
 * agreement-updates program first, on the website and in the app, and then
 * the forms that text one-time codes through Twilio Verify, apart; each
 * step with its picture, and the exact wording of each screen and message,
 * read without scripts, with nothing inline.
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

/** Whether a form texts a one-time code, and so is in the codes' section. */
const isCodeForm = (form: Form) => (CODE_FORMS as readonly Form[]).includes(form)

/** A form's section, under its heading's anchor: an h3 in a part, an h4 among the codes. */
function formSection(page: Document, form: Form, surface: Surface): Element {
  const heading = page.getElementById(formAnchor(form, surface))
  expect(heading?.tagName, formAnchor(form, surface)).toBe(isCodeForm(form) ? 'H4' : 'H3')
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

  test('the message Twilio Verify sends is its default template, with the services’ names', () => {
    expect(VERIFY_SAMPLE).toBe('Your Yuppers.app verification code is: 123456')
    // A deletion code says what it is for.
    expect(VERIFY_DELETION_SAMPLE).toBe(
      'Your Yuppers.app account deletion verification code is: 123456',
    )
    // The backend says the same of what it has Verify send.
    const verify = readFileSync(join(repoRoot, 'backend/src/notifications/verify.rs'), 'utf8')
    expect(verify).toContain('https://www.twilio.com/docs/verify/api/verification')
    // The deployment guide names the services so.
    const guide = readFileSync(join(repoRoot, 'docs/deploy-render.md'), 'utf8')
    expect(guide).toContain(VERIFY_SAMPLE)
    expect(guide).toContain(VERIFY_DELETION_SAMPLE)
  })

  test.each(['en', 'es'])(
    'in %s puts agreement updates first, on the website and in the app, and the code forms after, each under its anchor',
    (language) => {
      const page = parse(language)
      const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
      expect(page.documentElement.lang).toBe(language)
      expect(page.title).toBe(`${wording.title} · Yuppers`)
      const root = page.getElementById('root')!
      expect(root.querySelector('h1')?.textContent).toBe(wording.title)
      expect(root.textContent).toContain(wording.note)

      // The parts in order: the website's updates, the app's, then the codes.
      const article = page.querySelector('article')!
      expect([...article.querySelectorAll(':scope > section.part > h2')].map((h) => h.id)).toEqual([
        PART_ANCHORS.web,
        PART_ANCHORS.mobile,
        CODES_ANCHOR,
      ])
      const codes = page.getElementById(CODES_ANCHOR)!
      expect(codes.textContent).toBe(wording.codesHeading)
      expect(wording.codesHeading).toMatch(/Twilio Verify/)
      expect(codes.parentElement!.querySelector('h2 + p')?.textContent).toBe(wording.codesIntro)

      for (const surface of SURFACES) {
        const part = page.getElementById(PART_ANCHORS[surface])!
        expect(part.tagName).toBe('H2')
        expect(part.textContent).toBe(surface === 'web' ? wording.websiteHeading : wording.mobileHeading)
        const section = part.parentElement!
        expect(section.matches('section.part')).toBe(true)
        // The updates' forms, in order, and nothing that texts a code.
        expect([...section.querySelectorAll(':scope > section.form > h3')].map((h) => h.id)).toEqual(
          UPDATE_FORMS.map((form) => formAnchor(form, surface)),
        )
        // The code forms, in the codes' section, under this surface.
        const among = page.getElementById(codesAnchor(surface))!
        expect(among.tagName).toBe('H3')
        expect(among.textContent).toBe(
          surface === 'web' ? wording.codesWebsiteHeading : wording.codesMobileHeading,
        )
        expect(
          [...among.parentElement!.querySelectorAll(':scope > section.form > h4')].map((h) => h.id),
        ).toEqual(CODE_FORMS.map((form) => formAnchor(form, surface)))

        for (const form of FORM_NAMES) {
          const each = formSection(page, form, surface)
          const [formLevel, stepLevel] = isCodeForm(form) ? ['h4', 'h5'] : ['h3', 'h4']
          expect(each.querySelector(formLevel)?.textContent).toBe(wording.forms[form].heading)
          expect(each.querySelector(`${formLevel} + p`)?.textContent).toBe(wording.forms[form].intro)
          const steps = (surface === 'web' ? wording.steps : wording.mobileSteps) as SmsOptInWording['steps']
          expect(
            [...each.querySelectorAll(`section.step > ${stepLevel}`)].map((h) => [h.id, h.textContent]),
          ).toEqual(FORMS[form].map((step) => [stepAnchor(step, surface), steps[step].title]))
          for (const step of FORMS[form]) {
            const section = page.getElementById(stepAnchor(step, surface))!.parentElement!
            expect(section.querySelector(`${stepLevel} + p`)?.textContent).toBe(steps[step].caption)
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
      'one-time-codes',
      'website-one-time-codes',
      'mobile-app-one-time-codes',
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
    'in %s quotes Verify’s message for each code, the confirmation and the replies word for word as they are sent',
    (language) => {
      const page = parse(language)
      const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
      const product = read(`${language}.json`)
      // No text of the service's own carries a code any more.
      expect(Object.keys(product.sms).sort()).toEqual(['optInConfirmation', 'update'])
      for (const surface of SURFACES) {
        const after = (step: Screen) => quotedAt(page, step, surface)
        // Each "code sent" screen: what it says, then the message Twilio
        // Verify sends, said to be Verify's, with what it is.
        const verifySaid = (step: Screen, sample = VERIFY_SAMPLE, before = wording.verifyText) => {
          const section = page.getElementById(stepAnchor(step, surface))!.parentElement!
          expect(after(step).at(-1)).toBe(sample)
          expect(section.textContent).toContain(before)
          expect(section.textContent).toContain(wording.verifyNote)
          expect(section.querySelector('.sms figcaption')?.textContent).toBe(wording.messageFromVerify)
        }
        const signedIn = after('codeSent')
        expect(signedIn).toContain(fill(product.signIn.codeSent, { identifier: SAMPLE_PHONE }))
        verifySaid('codeSent')

        const confirmed = after('confirmNumberCodeSent')
        expect(confirmed).toContain(
          fill(product.smsUpdates.codeSent, { phone: masked(SAMPLE_PHONE_TO_ADD) }),
        )
        expect(confirmed).toContain(product.smsUpdates.codeLabel)
        verifySaid('confirmNumberCodeSent')

        const deleting = after('deleteAccount')
        expect(deleting).toContain(fill(product.deletion.codeIntro, { identifier: SAMPLE_PHONE }))
        const deleted = after('deleteAccountCodeSent')
        expect(deleted).toContain(fill(product.deletion.codeSent, { identifier: SAMPLE_PHONE }))
        verifySaid('deleteAccountCodeSent', VERIFY_DELETION_SAMPLE, wording.verifyDeletionText)

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

  test.each(['en', 'es'])('in %s opens with a contents list of every form in every part', (language) => {
    const page = parse(language)
    const wording = read(`sms-opt-in/${language}.json`) as SmsOptInWording
    const contents = page.querySelector(`nav[aria-label="${wording.contentsLabel}"]`)!
    const links = [...contents.querySelectorAll('a')]
    expect(links.map((link) => [link.getAttribute('href'), link.textContent])).toEqual([
      ...SURFACES.flatMap((surface) => [
        [`#${PART_ANCHORS[surface]}`, surface === 'web' ? wording.websiteHeading : wording.mobileHeading],
        ...UPDATE_FORMS.map((form) => [`#${formAnchor(form, surface)}`, wording.forms[form].heading]),
      ]),
      [`#${CODES_ANCHOR}`, wording.codesHeading],
      ...SURFACES.flatMap((surface) => [
        [
          `#${codesAnchor(surface)}`,
          surface === 'web' ? wording.codesWebsiteHeading : wording.codesMobileHeading,
        ],
        ...CODE_FORMS.map((form) => [`#${formAnchor(form, surface)}`, wording.forms[form].heading]),
      ]),
    ])
    // Before either part, and each link leads to a heading.
    const article = page.querySelector('article')!
    const first = article.querySelector('section.part')!
    expect(contents.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    for (const link of links) {
      const target = page.getElementById(link.getAttribute('href')!.slice(1))!
      expect(['H2', 'H3', 'H4']).toContain(target.tagName)
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
    // In the page's order: each part's updates, then each part's codes.
    const pictures = (forms: readonly Form[]) =>
      SURFACES.flatMap((surface) =>
        forms.flatMap((form) =>
          (FORMS[form] as readonly string[])
            .filter((step): step is Screen => step in SCREENSHOTS)
            .map((screen) => screenshotPath(screen, language, 'en', surface)),
        ),
      )
    expect(images.map((image) => image.getAttribute('src'))).toEqual([
      ...pictures(UPDATE_FORMS),
      ...pictures(CODE_FORMS),
    ])
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
    // The links to give for the program's opt-in, on the website and in
    // the app, are this page's anchors.
    expect(guide).toContain('https://yuppers.app/sms-opt-in#website-agreement-updates')
    expect(guide).toContain('https://yuppers.app/sms-opt-in#mobile-app-agreement-updates')
    for (const surface of SURFACES) {
      for (const form of UPDATE_FORMS) {
        expect(guide).toContain(`https://yuppers.app/sms-opt-in#${formAnchor(form, surface)}`)
      }
    }
    // The samples are the program's own texts, word for word: the update
    // and the confirmation in English, and the update in Spanish.
    const en = read('en.json').sms
    const es = read('es.json').sms
    for (const text of [
      fill(en.update, { link: SAMPLE_LINK }),
      en.optInConfirmation,
      fill(es.update, { link: SAMPLE_LINK }),
    ]) {
      expect(guide).toContain(text)
    }
  })
})
