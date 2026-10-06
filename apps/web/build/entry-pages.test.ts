import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, test } from 'vitest'

import { entryPages, invitationPath, readEntryPages, renderEntryPage } from './entry-pages.ts'

const wordingDirectory = fileURLToPath(new URL('../../../packages/shared/wording', import.meta.url))
const template = readFileSync(new URL('../index.html', import.meta.url), 'utf8')

const read = (name: string) =>
  JSON.parse(readFileSync(`${wordingDirectory}/${name}`, 'utf8')) as unknown
const languages = read('languages.json') as { code: string; direction: string }[]

describe('the pages built from the real wording', () => {
  const pages = readEntryPages(wordingDirectory)

  test('every supported language has an invitation page at /{language}/i', () => {
    for (const language of languages) {
      const page = pages.find((found) => found.fileName === `${language.code}/i/index.html`)
      expect(page, language.code).toBeDefined()
      expect(`/${page!.fileName}`).toBe(`${invitationPath(language.code)}/index.html`)
    }
    expect(pages).toHaveLength(languages.length + 1)
  })

  test('each previews in its own language, with that language’s wording', () => {
    for (const language of languages) {
      const wording = read(`${language.code}.json`) as {
        linkPreview: { title: string; description: string }
      }
      const page = pages.find((found) => found.fileName === `${language.code}/i/index.html`)!
      expect(page).toMatchObject({
        lang: language.code,
        dir: language.direction,
        title: wording.linkPreview.title,
        description: wording.linkPreview.description,
      })

      const html = renderEntryPage(template, page)
      expect(html).toContain(`<html lang="${language.code}" dir="${language.direction}">`)
      expect(html).toContain(`<meta property="og:title" content="${page.title}" />`)
      expect(html).toContain(`<meta property="og:description" content="${page.description}" />`)
      expect(html).toContain(`<meta name="description" content="${page.description}" />`)
      // An invitation is nobody's business but the two people's.
      expect(html).toContain('<meta name="robots" content="noindex" />')
      expect(html).not.toMatch(/\{\{\w+\}\}/)
    }
  })

  test('the app’s own entry page is in the default language', () => {
    expect(pages[0]).toMatchObject({ fileName: 'index.html', lang: languages[0].code })
  })

  test('the template itself says nothing in any language', () => {
    // Everything a preview shows comes from the wording files.
    expect(template).toContain('<title>{{title}}</title>')
    expect(template).toContain('<html lang="{{lang}}" dir="{{dir}}">')
  })
})

test('a language added to the wording gets its page with no change here', () => {
  const pages = entryPages(
    [
      { code: 'en', direction: 'ltr' },
      { code: 'ar', direction: 'rtl' },
    ],
    (code) => ({
      productName: 'Yuppers',
      tagline: `tagline ${code}`,
      linkPreview: { title: `title ${code}`, description: `description ${code}` },
    }),
  )
  expect(pages.map((page) => page.fileName)).toEqual([
    'index.html',
    'en/i/index.html',
    'ar/i/index.html',
  ])
  expect(pages[2]).toEqual({
    fileName: 'ar/i/index.html',
    lang: 'ar',
    dir: 'rtl',
    title: 'title ar',
    description: 'description ar',
    robots: 'noindex',
  })
  expect(renderEntryPage(template, pages[2])).toContain('<html lang="ar" dir="rtl">')
})

test('wording is escaped, so it cannot break out of the page', () => {
  const html = renderEntryPage(template, {
    fileName: 'x/i/index.html',
    lang: 'en',
    dir: 'ltr',
    title: 'Tom & "Jerry" <script>',
    description: 'a "quoted" line',
    robots: 'noindex',
  })
  expect(html).toContain('<title>Tom &amp; &quot;Jerry&quot; &lt;script&gt;</title>')
  expect(html).toContain('content="a &quot;quoted&quot; line"')
  expect(html).not.toContain('<script>')
})
