// The invitation page's size budget (DESIGN.md §13.5).
//
// An invitation link is the front door: the person opening it has no account
// and no reason to wait, often on a mid-range phone over a mobile connection.
// This adds up everything the page at `/{language}/i` loads before anyone can
// act on it, compressed as it travels, and fails when that grows past the
// budget in `budget.json`:
//
//   - the language's entry page, `{language}/i/index.html`;
//   - the entry script and every chunk it imports statically (the page names
//     them as modulepreload links), with their stylesheets;
//   - the one wording file the page fetches before its first render, and what
//     that imports.
//
// The fonts the page preloads (`build/font-preload.ts`, read from its
// `<link rel="preload" as="font">` links) load alongside and are held to a
// budget of their own, `fonts`, in bytes as stored: a WOFF2 file is already
// compressed, and gzip or brotli on the way gains nothing. They are kept
// apart because they do not hold up acting on the page (text shows in the
// system font until they arrive, `font-display: swap`), and so that what the
// code and the wording grow by is not hidden behind them. The fonts the page
// does not preload are not counted: they load once text in them is drawn.
//
// Each language is counted on its own and the largest is held to the budget.
// What loads only after someone acts (signing up, the composer, the other
// screens) is not counted. The favicon is left out: it does not hold up the
// page. The main app (the list of exchanges, signed in) is reported too, for
// information, with no budget.
//
// Run it after `npm run build:web`, which writes the build manifest it reads
// to `apps/web/.build/manifest.json` (see `build/manifest.ts`).
//
// Raising the budget is a decision, not a fix: when this fails, first look for
// what grew and whether the invitation page needs it (a screen or a library
// that could load later, wording that belongs to another screen). If the page
// does need it, raise `gzip` and `brotli` (or `fonts`) in `budget.json` in the
// same change, to the new size plus about 10%, and say why in the commit
// message.

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { brotliCompressSync, constants, gzipSync } from 'node:zlib'

const web = fileURLToPath(new URL('../', import.meta.url))
const dist = `${web}dist/`
const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'))

let manifest
try {
  manifest = readJson(`${web}.build/manifest.json`)
} catch {
  console.error('No build manifest at apps/web/.build/manifest.json. Run `npm run build:web` first.')
  process.exit(1)
}
const budget = readJson(`${web}budget.json`).invitation
const languages = readJson(`${web}../../packages/shared/wording/languages.json`).map(
  (language) => language.code,
)

/** Sizes of one built file: as stored, gzip at level 9 and brotli at quality 11. */
const sizes = new Map()
function measure(file) {
  if (!sizes.has(file)) {
    const bytes = readFileSync(dist + file)
    sizes.set(file, {
      raw: bytes.length,
      gzip: gzipSync(bytes, { level: 9 }).length,
      brotli: brotliCompressSync(bytes, {
        params: {
          [constants.BROTLI_PARAM_QUALITY]: 11,
          [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length,
        },
      }).length,
    })
  }
  return sizes.get(file)
}

/** A manifest chunk's file and stylesheets, and those of every chunk it imports statically. */
function closure(keys) {
  const files = new Set()
  const seen = new Set()
  const visit = (key) => {
    if (seen.has(key)) return
    seen.add(key)
    const chunk = manifest[key]
    if (!chunk) throw new Error(`build manifest has no ${key}`)
    files.add(chunk.file)
    for (const css of chunk.css ?? []) files.add(css)
    for (const imported of chunk.imports ?? []) visit(imported)
  }
  keys.forEach(visit)
  return files
}

const entry = Object.keys(manifest).find((key) => manifest[key].isEntry && key === 'index.html')
if (!entry) throw new Error('build manifest has no entry for index.html')

/** The chunk holding a language's wording, which `src/app/wording.ts` fetches. */
function wordingChunk(code) {
  const key = (manifest[entry].dynamicImports ?? []).find(
    (candidate) =>
      manifest[candidate]?.name === code || candidate.endsWith(`/wording/${code}.json`),
  )
  if (!key) throw new Error(`build manifest has no wording chunk for ${code}`)
  return key
}

/** The built files a page preloads as fonts, from its own `<link rel="preload" as="font">`. */
function preloadedFonts(page) {
  const html = readFileSync(dist + page, 'utf8')
  return [...html.matchAll(/<link\b[^>]*>/g)]
    .map(([link]) => link)
    .filter((link) => /\brel="preload"/.test(link) && /\bas="font"/.test(link))
    .map((link) => {
      const href = /\bhref="\/([^"]+)"/.exec(link)?.[1]
      if (!href) throw new Error(`${page} preloads a font with no same-origin href: ${link}`)
      return href
    })
}

function total(files) {
  const sum = { raw: 0, gzip: 0, brotli: 0 }
  for (const file of files) {
    const size = measure(file)
    sum.raw += size.raw
    sum.gzip += size.gzip
    sum.brotli += size.brotli
  }
  return sum
}

const kB = (bytes) => (bytes / 1024).toFixed(1)
function table(title, files) {
  const rows = [...files].map((file) => [file, measure(file)])
  rows.sort((a, b) => b[1].gzip - a[1].gzip)
  const sum = total(files)
  const width = Math.max(28, ...rows.map(([file]) => file.length))
  const line = (name, size) =>
    `  ${name.padEnd(width)} ${kB(size.raw).padStart(8)} ${kB(size.gzip).padStart(8)} ${kB(size.brotli).padStart(8)}`
  console.log(`\n${title}`)
  console.log(`  ${'file'.padEnd(width)} ${'raw kB'.padStart(8)} ${'gzip kB'.padStart(8)} ${'br kB'.padStart(8)}`)
  for (const [file, size] of rows) console.log(line(file, size))
  console.log(line('total', sum))
  return sum
}

const problems = []
let largest = { gzip: 0, brotli: 0, fonts: 0 }
for (const code of languages) {
  const page = `${code}/i/index.html`
  const files = new Set([page, ...closure([entry, wordingChunk(code)])])
  const sum = table(`Invitation page, /${code}/i, before interaction`, files)
  const fonts = table(`Invitation page, /${code}/i, preloaded fonts`, new Set(preloadedFonts(page)))
  largest = {
    gzip: Math.max(largest.gzip, sum.gzip),
    brotli: Math.max(largest.brotli, sum.brotli),
    fonts: Math.max(largest.fonts, fonts.raw),
  }
  for (const method of ['gzip', 'brotli']) {
    if (sum[method] > budget[method]) {
      problems.push(
        `/${code}/i is ${sum[method]} bytes with ${method}, over its budget of ${budget[method]}`,
      )
    }
  }
  if (fonts.raw > budget.fonts) {
    problems.push(`/${code}/i preloads ${fonts.raw} bytes of fonts, over their budget of ${budget.fonts}`)
  }
}

const [first] = languages
const home = Object.keys(manifest).find((key) => key.endsWith('src/screens/HomePage.tsx'))
if (home) {
  const files = new Set(['index.html', ...closure([entry, home, wordingChunk(first)])])
  table(`Main app, / signed in (${first}), for information: no budget`, files)
}

console.log(
  `\nBudget for the invitation page: ${budget.gzip} bytes gzip (largest ${largest.gzip}), ` +
    `${budget.brotli} bytes brotli (largest ${largest.brotli}); ` +
    `preloaded fonts ${budget.fonts} bytes (largest ${largest.fonts}).`,
)
if (problems.length > 0) {
  console.error(`\n${problems.join('\n')}`)
  console.error(
    'Find what grew first; raise apps/web/budget.json only if the invitation page needs it ' +
      '(see the top of apps/web/scripts/check-budget.mjs).',
  )
  process.exit(1)
}
