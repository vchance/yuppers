import helpEn from '../../wording/help/en.json'
import en from '../../wording/en.json'
import type { HelpWording, Wording } from '../wording/types'

/*
 * A pseudo-language, for tests only: nothing in the apps imports this, so it
 * is never built into what people get.
 *
 * Every message of the English wording is rewritten so that it still reads,
 * but with no plain Latin letter left in it, and a third longer:
 *
 *   `Your terms`  becomes  `[Ýöûŕ ţéŕɱš····]`
 *
 * Placeholders and the ICU plural syntax stay as they are, so the message
 * still fills in. A screen rendered with this wording should then show no
 * plain Latin letter at all, except in what people wrote themselves (names,
 * terms, notes) and what the platform formats (month names). Anything else
 * was written into a component instead of the wording, and would stay in
 * English whatever language the person reads (DESIGN.md §4.2). The brackets
 * show where a message starts and ends, so text cut off or glued together
 * stands out, and the padding stands in for a language that runs longer.
 */

const LOWER = 'áƀçďéƒĝĥíĵķĺɱñöþʠŕšţûṽŵẋýž'
const UPPER = 'ÁƁÇĎÉƑĜĤÍĴĶĹṀÑÖÞǪŔŠŢÛṼŴẊÝŽ'
const A = 'a'.charCodeAt(0)

function accent(text: string): string {
  return text.replace(/[A-Za-z]/g, (letter) => {
    const lower = letter.toLowerCase()
    const index = lower.charCodeAt(0) - A
    return letter === lower ? [...LOWER][index] : [...UPPER][index]
  })
}

/** The index of the brace that closes the one at `open`, or -1. */
function closing(message: string, open: number): number {
  let depth = 0
  for (let index = open; index < message.length; index += 1) {
    if (message[index] === '{') depth += 1
    else if (message[index] === '}') {
      depth -= 1
      if (depth === 0) return index
    }
  }
  return -1
}

/** Accents the text of a message, leaving `{name}` and `{count, plural, …}` working. */
function rewrite(message: string): string {
  let output = ''
  let index = 0
  while (index < message.length) {
    const open = message.indexOf('{', index)
    if (open === -1) {
      output += accent(message.slice(index))
      break
    }
    output += accent(message.slice(index, open))
    const end = closing(message, open)
    if (end === -1) {
      output += message.slice(open)
      break
    }
    output += placeholder(message.slice(open + 1, end))
    index = end + 1
  }
  return output
}

/** `name` stays; in `count, plural, one {…} other {…}` only the branches' text changes. */
function placeholder(inner: string): string {
  const parts = inner.split(',')
  if (parts.length < 3 || parts[1].trim() !== 'plural') return `{${inner}}`
  const head = `${parts[0]},${parts[1]},`
  let branches = inner.slice(head.length)
  let output = ''
  while (branches.length > 0) {
    const open = branches.indexOf('{')
    if (open === -1) {
      output += branches
      break
    }
    const end = closing(branches, open)
    if (end === -1) {
      output += branches
      break
    }
    output += `${branches.slice(0, open)}{${rewrite(branches.slice(open + 1, end))}}`
    branches = branches.slice(end + 1)
  }
  return `{${head}${output}}`
}

/** One message in the pseudo-language. */
export function pseudoMessage(message: string): string {
  const padding = '·'.repeat(Math.ceil(message.length * 0.3))
  return `[${rewrite(message)}${padding}]`
}

function transform<T>(value: T): T {
  if (typeof value === 'string') return pseudoMessage(value) as T
  if (Array.isArray(value)) return value.map(transform) as T
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([key, child]) => [key, transform(child)]),
  ) as T
}

/** The whole English wording in the pseudo-language. */
export function pseudoWording(): Wording {
  return transform(en as Wording)
}

/** The English help pages in the pseudo-language. */
export function pseudoHelp(): HelpWording {
  return transform(helpEn as HelpWording)
}

/**
 * What the platform writes in words when it formats a date or a time for
 * `language`, as the apps format them (`i18n.ts`, `record.ts`): month names,
 * day periods, the words joining a date to its time, and the names of the
 * time zones given. Numbers, currency symbols and punctuation have no Latin
 * letters to begin with.
 */
export function formattedWords(language: string, timeZones: readonly string[] = []): string[] {
  const words = new Set<string>()
  const styles: Intl.DateTimeFormatOptions[] = [
    { dateStyle: 'long' },
    { dateStyle: 'long', timeStyle: 'short' },
    { dateStyle: 'long', timeStyle: 'long' },
    { month: 'short' },
  ]
  const zones = ['UTC', Intl.DateTimeFormat().resolvedOptions().timeZone, ...timeZones]
  for (const timeZone of zones) {
    for (const style of styles) {
      const format = new Intl.DateTimeFormat(language, { ...style, timeZone })
      for (let month = 0; month < 12; month += 1) {
        for (const hour of [9, 21]) {
          for (const part of format.formatToParts(new Date(Date.UTC(2026, month, 15, hour)))) {
            if (/[A-Za-z]/.test(part.value)) words.add(part.value.trim())
          }
        }
      }
    }
  }
  return [...words]
}

/** Things that are not words in any language: hashes, ids, references, addresses. */
const NOT_WORDS = [
  /\bhttps?:\/\/\S+\/i#[\w-]+/g, // an invitation link, `{origin}/{language}/i#{token}`
  /\b[0-9a-f]{64}\b/g, // a content hash
  /\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b/g, // an id
  /\b[A-Z0-9]{2,4}-[A-Z0-9]{4}\b/g, // an exchange's reference, such as PVVS-5Q2K
  /\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b/g, // an email address
]

/**
 * The parts of `text` that hold a plain Latin letter and did not come from
 * the pseudo-language, once what is `allowed` (written by people, formatted
 * by the platform, or a language's own name) is taken out. Empty when the
 * text is all wording.
 */
export function untranslated(text: string, allowed: readonly string[]): string[] {
  let rest = text
  for (const pattern of NOT_WORDS) rest = rest.replace(pattern, ' ')
  // The longest first, so a phrase goes before a word inside it.
  for (const words of [...allowed].sort((a, b) => b.length - a.length)) {
    if (!words) continue
    const escaped = words.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    rest = rest.replace(new RegExp(`(?<![A-Za-z])${escaped}(?![A-Za-z])`, 'g'), ' ')
  }
  return rest.split(/\s+/).filter((word) => /[A-Za-z]/.test(word))
}
