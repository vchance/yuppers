// Checks the wording files against each other, independent of TypeScript:
// every language in the manifest has a file, and every file has exactly the
// keys of the reference language, each with text in it. The typecheck catches
// a missing key; this also catches a stray one, an empty one, and a language
// listed without a file.
//
// The help pages' text, in `wording/help/`, is checked the same way, as a set
// of its own. A help page is a list of headings, paragraphs and lists, and a
// list's entries are keys like any other (`blocks.3.ul.0`), so every language
// must have the same pieces in the same order.
//
// The page on opting in to texts, `wording/sms-opt-in/`, is checked the same
// way, as a set of its own.
//
// The privacy policy's text, in `wording/privacy/`, and the terms', in
// `wording/terms/`, are two more sets, checked the same way: every language
// has the same sections, under the same anchors, with the same pieces in the
// same order.

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const directory = fileURLToPath(new URL('../wording/', import.meta.url))
const read = (name) => JSON.parse(readFileSync(directory + name, 'utf8'))

/** Every path to a string in a wording file, such as `errors.NOT_FOUND`. */
function paths(value, prefix = '') {
  if (typeof value === 'string') return [[prefix, value]]
  return Object.entries(value).flatMap(([key, child]) =>
    paths(child, prefix ? `${prefix}.${key}` : key),
  )
}

/** The variables an ICU message uses, such as `count` in `{count, plural, …}`. */
function variables(message) {
  return [...message.matchAll(/\{\s*(\w+)/g)].map((found) => found[1]).sort().join(',')
}

const problems = []
const manifest = read('languages.json')
const codes = manifest.map((language) => language.code)
const [reference] = codes

for (const language of manifest) {
  if (!/^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$/.test(language.code)) {
    problems.push(`${language.code}: not a language tag`)
  }
  if (!language.name) problems.push(`${language.code}: no name`)
  if (!['ltr', 'rtl'].includes(language.direction)) {
    problems.push(`${language.code}: direction must be ltr or rtl`)
  }
}
if (new Set(codes).size !== codes.length) problems.push('languages.json lists a language twice')

/**
 * Checks one set of wording files, `folder` being '' for the product's
 * wording, 'help/' for the help pages', or 'privacy/' or 'terms/' for
 * the privacy policy's or the terms'. Returns how many messages each language has.
 */
function check(folder) {
  const label = (code) => `${folder}${code}`
  const files = readdirSync(directory + folder).filter(
    (name) => name !== 'languages.json' && statSync(directory + folder + name).isFile(),
  )
  for (const file of files) {
    if (!codes.includes(file.replace(/\.json$/, ''))) {
      problems.push(`${folder}${file}: not listed in languages.json`)
    }
  }

  let expected
  try {
    expected = new Map(paths(read(`${folder}${reference}.json`)))
  } catch {
    problems.push(`${label(reference)}: no readable wording/${folder}${reference}.json`)
    return 0
  }

  for (const code of codes) {
    let found
    try {
      found = new Map(paths(read(`${folder}${code}.json`)))
    } catch {
      problems.push(`${label(code)}: no readable wording/${folder}${code}.json`)
      continue
    }
    for (const [path, message] of expected) {
      const translated = found.get(path)
      if (translated === undefined) problems.push(`${label(code)}: missing ${path}`)
      else if (!translated.trim()) problems.push(`${label(code)}: empty ${path}`)
      else if (variables(translated) !== variables(message)) {
        problems.push(`${label(code)}: ${path} does not use the same variables as ${reference}`)
      }
    }
    for (const path of found.keys()) {
      if (!expected.has(path)) problems.push(`${label(code)}: unexpected ${path}`)
    }
  }
  return expected.size
}

const messages = check('')
const helpMessages = check('help/')
const privacyMessages = check('privacy/')
const termsMessages = check('terms/')
const optInMessages = check('sms-opt-in/')

if (problems.length > 0) {
  console.error(problems.join('\n'))
  process.exit(1)
}
console.log(
  `wording: ${codes.length} languages, ${messages} messages each, ${helpMessages} in help, ` +
    `${privacyMessages} in the privacy policy, ${termsMessages} in the terms and ` +
    `${optInMessages} on the page on opting in to texts`,
)
