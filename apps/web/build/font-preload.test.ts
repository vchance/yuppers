/// <reference types="node" />
import { readFileSync } from 'node:fs'

import { expect, test } from 'vitest'

import { PRELOADED_FONTS, preloadedFontHrefs, withFontPreloads } from './font-preload.ts'

const assets = [
  { fileName: 'assets/index-abc.css', originalFileNames: ['src/index.css'] },
  { fileName: 'assets/Gabarito-Black-B1.woff2', originalFileNames: [PRELOADED_FONTS[1]] },
  { fileName: 'assets/InstrumentSans-Regular-A1.woff2', originalFileNames: [PRELOADED_FONTS[0]] },
  { fileName: 'assets/InstrumentSans-Bold-C1.woff2', originalFileNames: ['src/fonts/instrument-sans/InstrumentSans-Bold.woff2'] },
]

test('finds each preloaded font by its source, in order, with its hash', () => {
  expect(preloadedFontHrefs(assets)).toEqual([
    '/assets/InstrumentSans-Regular-A1.woff2',
    '/assets/Gabarito-Black-B1.woff2',
  ])
})

test('fails the build when a preloaded font is not emitted', () => {
  expect(() => preloadedFontHrefs(assets.filter((asset) => !asset.fileName.includes('Gabarito')))).toThrow(
    /Gabarito-Black/,
  )
})

test('preloads each font after the stylesheet, as a font, in CORS mode', () => {
  const html = [
    '<html>',
    '  <head>',
    '    <link rel="stylesheet" crossorigin href="/assets/index-abc.css">',
    '  </head>',
    '</html>',
  ].join('\n')
  expect(withFontPreloads(html, ['/assets/a.woff2', '/assets/b.woff2'])).toBe(
    [
      '<html>',
      '  <head>',
      '    <link rel="stylesheet" crossorigin href="/assets/index-abc.css">',
      '    <link rel="preload" as="font" type="font/woff2" crossorigin href="/assets/a.woff2">',
      '    <link rel="preload" as="font" type="font/woff2" crossorigin href="/assets/b.woff2">',
      '  </head>',
      '</html>',
    ].join('\n'),
  )
})

test("the service's Content-Security-Policy lets a page load fonts from its own origin", () => {
  const policy = readFileSync(new URL('../../../backend/src/http/mod.rs', import.meta.url), 'utf8')
  const value = /CONTENT_SECURITY_POLICY_VALUE: &str = "([^;]+);/.exec(policy)?.[1]
  expect(value).toBe("default-src 'self'")
  expect(policy).not.toMatch(/font-src/)
})
