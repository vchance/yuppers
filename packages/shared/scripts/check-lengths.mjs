// How long each language's wording runs against English, and whether the
// messages that sit in tight spaces still fit.
//
// Spanish, and most languages after it, runs longer than English: a fifth to
// a third longer is usual, and a short label can double ("OK", "Entendido").
// Most of the wording sits in paragraphs that wrap, where that costs nothing.
// Some sits where there is no room to wrap: a button, a navigation title, a
// status tag, a line of a card in the mobile list, a store listing field.
//
// Two things happen here, for every language other than English:
//
//   1. A report, for information, of the messages that run much longer than
//      their English (at least LONGER_RATIO times as long, and LONGER_BY
//      characters more). Nothing fails on it; it is where to look when a
//      screen feels crowded in that language.
//
//   2. A failure when a message in a tight space is longer than its group
//      allows, in any language, English included. The groups and their keys
//      are listed in TIGHT below, by path, with `*` for every key at that
//      level. A placeholder such as `{name}` counts for nothing: what is
//      measured is the fixed text around it.
//
//      A new button, title, tag or card line goes into its group here. If a
//      message does not fit, shorten it in that language first; raise a
//      group's limit only if the screens have been checked in the longest
//      language at a narrow width (320 CSS pixels on the web, the smallest
//      phone on mobile) and still fit.
//
// The store listing drafts in `docs/mobile-release.md` are checked too: every
// line written `- **Field** (Store, N): text` must be at most N characters.

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const LONGER_RATIO = 1.5
const LONGER_BY = 10
/** How many of the longest-running messages the report lists. */
const REPORT = 15

const TIGHT = {
  // A button holds one or two lines at 320 CSS pixels. Link-style buttons
  // that read as a sentence and wrap, such as signIn.changeIdentifier, are
  // not here.
  'button labels': {
    max: 40,
    keys: [
      'common.cancel',
      'common.tryAgain',
      'common.goHome',
      'nav.signOut',
      'signIn.sendCode',
      'signIn.submit',
      'signIn.resend',
      'profile.continue',
      'profile.save',
      'invitation.respondNew',
      'invitationLink.copy',
      'invitationLink.share',
      'invitationLink.shareEmail',
      'invitationLink.shareSms',
      'invitationLink.shareWhatsApp',
      'invitationLink.shareQr',
      'invitationLink.hideQr',
      'invitationLink.closeShare',
      'invitationLink.reissue',
      'composer.addYours',
      'composer.addTheirs',
      'composer.review',
      'composer.backToEdit',
      'composer.discard',
      'composer.signAndSend',
      'exchange.accept',
      'exchange.decline',
      'exchange.withdraw',
      'exchange.counter',
      'exchange.amend',
      'exchange.refresh',
      'exchange.proposeEnd',
      'exchange.requestClose',
      'exchange.moves.*',
      'exchange.moneyMoves.*',
      'deletion.open',
      'deletion.sendCode',
      'deletion.continue',
      'deletion.dismiss',
      'record.open',
      'record.back',
      'record.download',
      'record.summary.savePdf',
      'safety.report',
      'safety.reportProposal',
      'trouble.open',
      'trouble.change',
      'mobile.back',
      'mobile.record.savePdf',
      'mobile.record.share',
      'mobile.openInvitation.title',
      'mobile.openInvitation.open',
      'mobile.openInvitation.paste',
      'mobile.notifications.turnOn',
      'mobile.notifications.notNow',
      'mobile.notifications.openSettings',
      'wallet.addToApple',
      'wallet.addToGoogle',
      'wallet.adding',
      'smsUpdates.sendCode',
      'smsUpdates.addPhone',
      'smsUpdates.save',
      'staff.outcomes.*',
      'staff.confirm',
      'staff.lift',
      'staff.restore',
    ],
  },
  // One line in the web header or the mobile navigation bar.
  'navigation and screen titles': {
    max: 24,
    keys: [
      'productName',
      'nav.exchanges',
      'nav.account',
      'nav.language',
      'home.title',
      'profile.title',
      'exchange.titleNoName',
      'common.notFoundTitle',
      'help.link',
    ],
  },
  // The tag beside an exchange in the list and on its page. An item's own
  // status (contributionStatus, moneyStatus) is a sentence that wraps. A
  // Wallet pass's status is its largest field, on a card the width of a
  // phone (backend/src/wallet/pass.rs).
  'status tags': {
    max: 28,
    keys: ['states.*', 'outcomes.*', 'wallet.status.*', 'staff.overdue', 'staff.standing.*'],
  },
  // A Wallet pass's labels, above their values in rows of two or three.
  'wallet pass labels': {
    max: 20,
    keys: [
      'wallet.pass.status',
      'wallet.pass.reference',
      'wallet.pass.nextDue',
      'wallet.pass.outstanding',
      'wallet.pass.with',
      'wallet.pass.closedOn',
    ],
  },
  // Each line of an exchange's card in the mobile list, beside its tag.
  'mobile home card lines': {
    max: 32,
    keys: ['home.withParty', 'home.noParty', 'home.reference', 'home.updated'],
  },
}

const shared = fileURLToPath(new URL('../', import.meta.url))
const read = (name) => JSON.parse(readFileSync(`${shared}wording/${name}`, 'utf8'))

/** Every path to a string in a wording file, such as `errors.NOT_FOUND`. */
function paths(value, prefix = '') {
  if (typeof value === 'string') return [[prefix, value]]
  return Object.entries(value).flatMap(([key, child]) =>
    paths(child, prefix ? `${prefix}.${key}` : key),
  )
}

/** The fixed text of a message: placeholders count for nothing. */
const fixed = (message) => [...message.replace(/\{[^{}]*\}/g, '')].length

/** Every path a key in TIGHT names: itself, or each one `*` stands for. */
function expand(key, all) {
  if (!key.endsWith('.*')) return all.has(key) ? [key] : []
  const prefix = key.slice(0, -1)
  return [...all.keys()].filter(
    (path) => path.startsWith(prefix) && !path.slice(prefix.length).includes('.'),
  )
}

const problems = []
const manifest = read('languages.json')
const [reference, ...others] = manifest.map((language) => language.code)
const english = new Map(paths(read(`${reference}.json`)))
const wording = new Map(
  manifest.map(({ code }) => [code, code === reference ? english : new Map(paths(read(`${code}.json`)))]),
)

for (const [group, { max, keys }] of Object.entries(TIGHT)) {
  for (const key of keys) {
    const found = expand(key, english)
    if (found.length === 0) problems.push(`${group}: ${key} is not in the wording`)
    for (const path of found) {
      for (const [code, messages] of wording) {
        const message = messages.get(path)
        if (message !== undefined && fixed(message) > max) {
          problems.push(
            `${code}: ${path} is ${fixed(message)} characters, over the ${max} for ${group}: “${message}”`,
          )
        }
      }
    }
  }
}

for (const code of others) {
  const messages = wording.get(code)
  const longer = []
  for (const [path, message] of english) {
    const translated = messages.get(path)
    if (translated === undefined) continue
    const ratio = translated.length / message.length
    if (ratio >= LONGER_RATIO && translated.length - message.length >= LONGER_BY) {
      longer.push({ path, ratio, from: message.length, to: translated.length })
    }
  }
  longer.sort((a, b) => b.ratio - a.ratio || b.to - a.to)
  const total = (map) => [...map.values()].reduce((sum, message) => sum + message.length, 0)
  console.log(
    `lengths: ${code} runs ${Math.round((total(messages) / total(english) - 1) * 100)}% longer than ${reference} overall; ` +
      `${longer.length} messages at least ${LONGER_RATIO}× and ${LONGER_BY} characters longer`,
  )
  for (const { path, ratio, from, to } of longer.slice(0, REPORT)) {
    console.log(`  ${ratio.toFixed(2)}×  ${String(from).padStart(3)} → ${String(to).padStart(3)}  ${path}`)
  }
}

// The store listing drafts: `- **Subtitle** (App Store, 30): Send a yup. Get it in writing.`
const listing = readFileSync(`${shared}../../docs/mobile-release.md`, 'utf8')
let fields = 0
for (const line of listing.split('\n')) {
  const field = /^- \*\*(.+?)\*\* \((.+?), (\d+)\): (.+)$/.exec(line)
  if (!field) continue
  fields += 1
  const [, name, store, limit, text] = field
  if ([...text].length > Number(limit)) {
    problems.push(
      `docs/mobile-release.md: ${name} (${store}) is ${[...text].length} characters, over its ${limit}`,
    )
  }
}
if (fields === 0) problems.push('docs/mobile-release.md: no store listing fields found to check')

if (problems.length > 0) {
  console.error(problems.join('\n'))
  process.exit(1)
}
console.log(
  `lengths: ${Object.keys(TIGHT).length} groups of tight messages fit in every language; ` +
    `${fields} store listing fields within their limits`,
)
