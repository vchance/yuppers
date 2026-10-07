/// <reference types="node" />
import { readdirSync, readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { brotliDecompressSync } from 'node:zlib'

import ts from 'typescript'
import { describe, expect, test } from 'vitest'

import { formatMoney, LAUNCH_CURRENCY } from '../../../packages/shared/src/decimal.ts'
import { PRELOADED_FONTS } from './font-preload.ts'

/*
 * The web fonts are subset to the characters the app writes in
 * (`src/fonts/README.md`). A character outside the subset is drawn in the
 * system font, in the middle of a word, with no error anywhere. So this reads
 * the characters each font has from the font files themselves and fails if
 * anything the screens can show is not in every one of them: the wording,
 * the text in the web app's source and in the static pages' builders, what a
 * stylesheet writes with `content`, and money and dates as each language
 * formats them. What people type themselves (names, notes) is theirs and is
 * not checked: anything outside the subset falls back to the system font.
 */

const web = fileURLToPath(new URL('../', import.meta.url))
const repo = join(web, '../..')
const wording = join(repo, 'packages/shared/wording')

function walk(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? walk(path) : [path]
  })
}

// ---- The characters a WOFF2 file maps -------------------------------------

/** WOFF2's UIntBase128: up to five bytes, seven bits each, high bit for more. */
function base128(bytes: Buffer, at: { offset: number }): number {
  let value = 0
  for (let index = 0; index < 5; index += 1) {
    const byte = bytes[at.offset++]
    value = value * 128 + (byte & 0x7f)
    if ((byte & 0x80) === 0) return value
  }
  throw new Error('a UIntBase128 longer than five bytes')
}

/** The code points a WOFF2 font's `cmap` maps to a glyph (W3C WOFF2 §4–5). */
function woff2CodePoints(file: Buffer): Set<number> {
  if (file.toString('latin1', 0, 4) !== 'wOF2') throw new Error('not a WOFF2 file')
  if (file.toString('latin1', 4, 8) === 'ttcf') throw new Error('a font collection')
  const numTables = file.readUInt16BE(12)
  const at = { offset: 48 }
  const tables: { tag: string; length: number }[] = []
  for (let index = 0; index < numTables; index += 1) {
    const flags = file[at.offset++]
    // Index 0 of the known tags is cmap, 10 glyf and 11 loca; the rest only need a length.
    let tag = ['cmap', ...Array<string>(9).fill(''), 'glyf', 'loca'][flags & 0x3f] ?? ''
    if ((flags & 0x3f) === 0x3f) {
      tag = file.toString('latin1', at.offset, at.offset + 4)
      at.offset += 4
    }
    const transform = flags >> 6
    const origLength = base128(file, at)
    const transformed = tag === 'glyf' || tag === 'loca' ? transform !== 3 : transform !== 0
    tables.push({ tag, length: transformed ? base128(file, at) : origLength })
  }
  const totalCompressedSize = file.readUInt32BE(20)
  const data = brotliDecompressSync(file.subarray(at.offset, at.offset + totalCompressedSize))
  let offset = 0
  for (const table of tables) {
    if (table.tag === 'cmap') return cmapCodePoints(data.subarray(offset, offset + table.length))
    offset += table.length
  }
  throw new Error('no cmap table')
}

function cmapCodePoints(cmap: Buffer): Set<number> {
  const points = new Set<number>()
  const count = cmap.readUInt16BE(2)
  for (let index = 0; index < count; index += 1) {
    const record = 4 + index * 8
    const platform = cmap.readUInt16BE(record)
    const encoding = cmap.readUInt16BE(record + 2)
    const unicode = platform === 0 || (platform === 3 && (encoding === 1 || encoding === 10))
    if (!unicode) continue
    const table = cmap.subarray(cmap.readUInt32BE(record + 4))
    const format = table.readUInt16BE(0)
    if (format === 4) {
      const segments = table.readUInt16BE(6) / 2
      const ends = 14
      const starts = ends + segments * 2 + 2
      const deltas = starts + segments * 2
      const ranges = deltas + segments * 2
      for (let segment = 0; segment < segments; segment += 1) {
        const end = table.readUInt16BE(ends + segment * 2)
        const start = table.readUInt16BE(starts + segment * 2)
        const delta = table.readInt16BE(deltas + segment * 2)
        const rangeAt = ranges + segment * 2
        const range = table.readUInt16BE(rangeAt)
        for (let code = start; code <= end && code !== 0xffff; code += 1) {
          let glyph: number
          if (range === 0) glyph = (code + delta) & 0xffff
          else {
            glyph = table.readUInt16BE(rangeAt + range + (code - start) * 2)
            if (glyph !== 0) glyph = (glyph + delta) & 0xffff
          }
          if (glyph !== 0) points.add(code)
        }
      }
    } else if (format === 12) {
      const groups = table.readUInt32BE(12)
      for (let group = 0; group < groups; group += 1) {
        const at = 16 + group * 12
        const start = table.readUInt32BE(at)
        const end = table.readUInt32BE(at + 4)
        const glyph = table.readUInt32BE(at + 8)
        for (let code = start; code <= end; code += 1) {
          if (glyph + (code - start) !== 0) points.add(code)
        }
      }
    }
  }
  return points
}

// ---- The characters the app writes ----------------------------------------

/** Where each character comes from, for the failure message. */
const required = new Map<number, Set<string>>()
function need(text: string, source: string) {
  for (const character of text) {
    const code = character.codePointAt(0)!
    // Line breaks and tabs are layout, not glyphs.
    if (code < 0x20) continue
    if (!required.has(code)) required.set(code, new Set())
    required.get(code)!.add(source)
  }
}

function strings(value: unknown, found: string[] = []): string[] {
  if (typeof value === 'string') found.push(value)
  else if (Array.isArray(value)) value.forEach((item) => strings(item, found))
  else if (value && typeof value === 'object') Object.values(value).forEach((item) => strings(item, found))
  return found
}

// The wording, every language and document. A language's own file loses the
// emails and the texts, which no screen shows, as it does in the build
// (`serviceWordingOut` in `vite.config.ts`).
for (const path of walk(wording).filter((file) => file.endsWith('.json'))) {
  const parsed = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>
  if (relative(wording, path).match(/^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*\.json$/)) {
    delete parsed.notifications
    delete parsed.sms
  }
  for (const text of strings(parsed)) need(text, relative(repo, path))
}

// Text in source: string literals, template literals and JSX text, but not
// comments, which nobody sees. The web app, the static pages' builders, and
// the shared code that writes text for screens; not tests, nor the fake
// service and the pseudo-language they use.
const sources = [
  ...walk(join(web, 'src')),
  ...walk(join(web, 'build')),
  ...walk(join(repo, 'packages/shared/src')),
].filter(
  (path) =>
    /\.tsx?$/.test(path) &&
    !/\.test\.tsx?$/.test(path) &&
    !/\.d\.ts$/.test(path) &&
    !path.includes('/src/test/') &&
    !path.includes('/src/testing/'),
)
for (const path of sources) {
  const file = ts.createSourceFile(path, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, false)
  const visit = (node: ts.Node) => {
    if (
      ts.isStringLiteral(node) ||
      ts.isNoSubstitutionTemplateLiteral(node) ||
      ts.isTemplateHead(node) ||
      ts.isTemplateMiddle(node) ||
      ts.isTemplateTail(node) ||
      ts.isJsxText(node)
    ) {
      need(node.text, relative(repo, path))
    }
    ts.forEachChild(node, visit)
  }
  visit(file)
}

// What stylesheets write with `content`, CSS escapes and all.
for (const path of [...walk(join(web, 'src')), ...walk(join(web, 'public'))].filter((file) =>
  file.endsWith('.css'),
)) {
  const css = readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '')
  for (const [, value] of css.matchAll(/\bcontent\s*:\s*([^;}]+)/g)) {
    for (const [, , text] of value.matchAll(/(['"])((?:\\.|(?!\1).)*)\1/g)) {
      need(
        text.replace(/\\([0-9a-fA-F]{1,6}) ?|\\(.)/g, (_, hex: string, other: string) =>
          hex ? String.fromCodePoint(parseInt(hex, 16)) : other,
        ),
        relative(repo, path),
      )
    }
  }
}

// Money, dates and numbers as each language writes them (`i18n.ts`).
const languages = JSON.parse(readFileSync(join(wording, 'languages.json'), 'utf8')) as {
  code: string
}[]
for (const { code } of languages) {
  for (const currency of [LAUNCH_CURRENCY, 'EUR', 'GBP']) {
    need(formatMoney(-123456789, currency, code), `formatMoney(${currency}, ${code})`)
  }
  const day = new Intl.DateTimeFormat(code, { dateStyle: 'long', timeZone: 'UTC' })
  const moment = new Intl.DateTimeFormat(code, {
    dateStyle: 'long',
    timeStyle: 'short',
    timeZone: 'UTC',
  })
  for (let month = 0; month < 12; month += 1) {
    for (const hour of [9, 21]) {
      const date = new Date(Date.UTC(2026, month, 28, hour, 30))
      need(day.format(date), `dates (${code})`)
      need(moment.format(date), `dates (${code})`)
    }
  }
}

// ---- The fonts ------------------------------------------------------------

const fonts = walk(join(web, 'src/fonts')).filter((path) => path.endsWith('.woff2'))

function name(code: number): string {
  return `U+${code.toString(16).toUpperCase().padStart(4, '0')} ${JSON.stringify(String.fromCodePoint(code))}`
}

test('there are web fonts to check, and the preloaded ones are among them', () => {
  expect(fonts.length).toBe(6)
  for (const preloaded of PRELOADED_FONTS) {
    expect(fonts.map((path) => relative(web, path))).toContain(preloaded)
  }
})

test('the app writes characters beyond ASCII, so the check is reading the sources', () => {
  for (const character of 'áéíóúñ¿«»’“”…•€') {
    expect(required.has(character.codePointAt(0)!), character).toBe(true)
  }
})

describe.each(fonts.map((path) => [relative(web, path), path]))('%s', (_, path) => {
  const points = woff2CodePoints(readFileSync(path))

  test('has every character the app writes', () => {
    const missing = [...required.keys()]
      .filter((code) => !points.has(code))
      .sort((a, b) => a - b)
      .map((code) => `${name(code)} in ${[...required.get(code)!].slice(0, 3).join(', ')}`)
    expect(missing, 'add them to UNICODES in scripts/subset-fonts.sh and run it again').toEqual([])
  })

  test('is subset: Latin and punctuation, nothing from other scripts', () => {
    expect(points.has(0x00f1)).toBe(true) // ñ
    expect(points.has(0x0153)).toBe(true) // œ, Latin Extended-A
    expect(points.has(0x2122)).toBe(true) // ™
    expect([...points].filter((code) => code >= 0x0370 && code < 0x2000)).toEqual([])
  })
})
