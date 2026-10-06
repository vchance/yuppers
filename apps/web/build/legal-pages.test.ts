// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import axe from 'axe-core'
import { describe, expect, test } from 'vitest'

import {
  LEGAL_DOCUMENTS,
  LEGAL_SECTIONS,
  PRIVACY_EMAIL,
  SUPPORT_EMAIL,
  type LegalDocument,
} from '../../../packages/shared/src/legal-text.ts'
import { renderEntryPage } from './entry-pages.ts'
import { legalPagePath, readLegalPages, withDocument } from './legal-pages.ts'
import { smsRequirements } from '../src/test/sms-requirements.ts'

// Under jsdom, `import.meta.url` is not a file's; the directory still is.
const wordingDirectory = join(import.meta.dirname, '../../../packages/shared/wording')
const template = readFileSync(join(import.meta.dirname, '../index.html'), 'utf8')
const read = (name: string) =>
  JSON.parse(readFileSync(`${wordingDirectory}/${name}`, 'utf8')) as unknown
const languages = read('languages.json') as { code: string; name: string; direction: string }[]
const codes = languages.map((language) => language.code)

const pages = readLegalPages(wordingDirectory)
const pageOf = (document: LegalDocument, language: string) =>
  pages.find((page) => page.document === document && page.lang === language)!
const html = (document: LegalDocument, language: string) =>
  withDocument(renderEntryPage(template, pageOf(document, language)), pageOf(document, language))
const parse = (document: LegalDocument, language: string) =>
  new DOMParser().parseFromString(html(document, language), 'text/html')

const cases = LEGAL_DOCUMENTS.flatMap((document) => codes.map((code) => [document, code] as const))

describe('the static pages of the privacy policy and the terms', () => {
  test('one per document and language: /{document} in the default, /{language}/{document} in the others', () => {
    expect(pages.map((page) => [page.path, page.fileName])).toEqual([
      ['/privacy', 'privacy/index.html'],
      ['/es/privacy', 'es/privacy/index.html'],
      ['/terms', 'terms/index.html'],
      ['/es/terms', 'es/terms/index.html'],
    ])
    expect(legalPagePath('terms', 'en', 'en')).toBe('/terms')
    expect(legalPagePath('privacy', 'es', 'en')).toBe('/es/privacy')
  })

  test.each(cases)(
    '%s in %s reads in full without scripts: every section, under its anchor, in order',
    (document, language) => {
      const wording = read(`${document}/${language}.json`) as {
        title: string
        description: string
        sections: Record<string, { title: string }>
      }
      const page = parse(document, language)
      expect(page.documentElement.lang).toBe(language)
      expect(page.title).toBe(`${wording.title} · Yuppers`)
      expect(page.querySelector('meta[name="description"]')?.getAttribute('content')).toBe(
        wording.description,
      )
      // Unlike every other page of the app, these are for search engines.
      expect(page.querySelector('meta[name="robots"]')?.getAttribute('content')).toBe(
        'index, follow',
      )

      const root = page.getElementById('root')!
      expect(root.querySelectorAll('h1')).toHaveLength(1)
      expect(root.querySelector('h1')?.textContent).toBe(wording.title)
      expect(root.querySelectorAll('main')).toHaveLength(1)
      const anchors = [...root.querySelectorAll('section > h2')].map((heading) => heading.id)
      expect(anchors).toEqual([...LEGAL_SECTIONS[document]])
      for (const id of LEGAL_SECTIONS[document]) {
        const title = wording.sections[id].title
        expect(page.getElementById(id)?.textContent).toBe(title)
        expect(root.querySelector(`nav a[href="#${id}"]`)?.textContent).toBe(title)
      }
      const email = document === 'privacy' ? PRIVACY_EMAIL : SUPPORT_EMAIL
      expect(root.querySelector(`a[href="mailto:${email}"]`)?.textContent).toBe(email)
      expect(root.querySelector('time')?.getAttribute('datetime')).toBe('2026-10-05')
      expect(root.textContent).not.toMatch(/[{}]|\*\*/)
      // Each links to the other.
      const other = document === 'privacy' ? 'terms' : 'privacy'
      expect(root.querySelector(`main a[href^="${legalPagePath(other, language, 'en')}"]`)).not.toBeNull()
    },
  )

  test.each(cases)('%s in %s says what the SMS registration asks for', (document, language) => {
    for (const problem of smsRequirements(parse(document, language), document, language)) {
      expect.fail(problem)
    }
  })

  test('each links to the other languages, to help, home, and both documents', () => {
    const page = parse('terms', 'es')
    const links = [...page.querySelectorAll('a')].map((link) => link.getAttribute('href'))
    expect(links).toEqual(
      expect.arrayContaining(['/', '#content', '/terms', '/es/terms', '/es/privacy', '/help?lang=es']),
    )
    const current = page.querySelectorAll('[aria-current="page"]')
    expect([...current].map((link) => link.getAttribute('href'))).toEqual(['/es/terms', '/es/terms'])
  })

  test('the app still starts on them, and nothing is inline', () => {
    for (const [document, language] of cases) {
      const page = html(document, language)
      // The same script and stylesheet as every page: the template's, untouched.
      expect(page).toContain('<script type="module" src="/src/main.tsx"></script>')
      expect(page.match(/<script/g)).toHaveLength(1)
      expect(page).not.toMatch(/<style|\sstyle=|\son[a-z]+=/i)
      expect(page).not.toMatch(/\{\{\w+\}\}/)
    }
  })

  test.each(cases)('%s in %s passes axe as it is served, before any script runs', async (document, language) => {
    const page = parse(document, language)
    window.document.documentElement.lang = page.documentElement.lang
    window.document.title = page.title
    window.document.head.innerHTML = page.head.innerHTML
    // The page as the service sends it, without the app's script running.
    window.document.body.innerHTML = page.body.innerHTML.replace(/<script[^>]*><\/script>/g, '')
    const results = await axe.run(window.document, {
      runOnly: {
        type: 'tag',
        values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'],
      },
      // Needs layout and painting, which jsdom does not do.
      rules: { 'color-contrast': { enabled: false } },
    })
    expect(
      results.violations.map(
        (violation) => `${violation.id}: ${violation.nodes.map((node) => node.html).join(' ')}`,
      ),
    ).toEqual([])
  })

  test('the wording is escaped', () => {
    const page = pageOf('privacy', 'en')
    const tricky = { ...page, title: '<b>&' }
    expect(withDocument(renderEntryPage(template, tricky), tricky)).toContain('&lt;b&gt;&amp;')
  })

  test('a build whose entry page has no empty root is refused', () => {
    expect(() => withDocument('<html></html>', pageOf('terms', 'en'))).toThrow(/root/)
  })
})
