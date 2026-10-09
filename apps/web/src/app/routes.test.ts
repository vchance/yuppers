import { expect, test } from 'vitest'

import { invitationToken, matchRoute, paths } from './routes'

const ID = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70'

test('an exchange lives at /exchanges/{id}, where notification emails link', () => {
  expect(matchRoute(`/exchanges/${ID}`)).toEqual({ name: 'exchange', id: ID })
  expect(matchRoute(paths.exchange(ID))).toEqual({ name: 'exchange', id: ID })
  expect(matchRoute(`/exchanges/${ID.toUpperCase()}/`)).toEqual({ name: 'exchange', id: ID })
  expect(matchRoute(paths.revise(ID))).toEqual({ name: 'revise', id: ID })
  expect(matchRoute(paths.record(ID))).toEqual({ name: 'record', id: ID })
  expect(matchRoute(`/exchanges/${ID}/record/`)).toEqual({ name: 'record', id: ID })
})

test('an invitation page is /{language}/i for any language tag', () => {
  expect(matchRoute('/en/i')).toEqual({ name: 'invitation', language: 'en' })
  expect(matchRoute('/pt-BR/i')).toEqual({ name: 'invitation', language: 'pt-BR' })
  expect(matchRoute('/zh-Hant/i/')).toEqual({ name: 'invitation', language: 'zh-Hant' })
})

test('help is /help, and each topic /help/{topic}', () => {
  expect(matchRoute('/help')).toEqual({ name: 'help', topic: null })
  expect(matchRoute('/help/')).toEqual({ name: 'help', topic: null })
  expect(matchRoute(paths.help())).toEqual({ name: 'help', topic: null })
  expect(matchRoute(paths.help('signing'))).toEqual({ name: 'help', topic: 'signing' })
  expect(matchRoute('/help/what-we-dont-do/')).toEqual({ name: 'help', topic: 'what-we-dont-do' })
  // A topic that does not exist is the help page's to say so.
  expect(matchRoute('/help/no-such-topic')).toEqual({ name: 'help', topic: 'no-such-topic' })
})

test('the privacy policy and the terms are /{document}, and /{language}/{document} in the others', () => {
  expect(paths.legal('privacy', 'en')).toBe('/privacy')
  expect(paths.legal('terms', 'es')).toBe('/es/terms')
  for (const document of ['privacy', 'terms'] as const) {
    for (const path of [`/${document}`, `/${document}/`, `/es/${document}`, `/en/${document}`]) {
      expect(matchRoute(path), path).toEqual({ name: 'legal', document })
    }
    for (const path of [`/fr/${document}`, `/${document}/more`, `/es/${document}/more`]) {
      expect(matchRoute(path), path).toEqual({ name: 'notFound' })
    }
  }
})

test('staff review is /staff, and one report /staff/reports/{id}', () => {
  expect(matchRoute(paths.staff)).toEqual({ name: 'staff' })
  expect(matchRoute('/staff/')).toEqual({ name: 'staff' })
  expect(matchRoute(paths.staffReport(ID))).toEqual({ name: 'staffReport', id: ID })
  expect(matchRoute(`/staff/reports/${ID.toUpperCase()}`)).toEqual({ name: 'staffReport', id: ID })
  for (const path of ['/staff/reports', '/staff/reports/not-an-id', `/staff/${ID}`]) {
    expect(matchRoute(path), path).toEqual({ name: 'notFound' })
  }
})

test('the other pages, and everything else', () => {
  expect(matchRoute('/')).toEqual({ name: 'home' })
  expect(matchRoute('/account')).toEqual({ name: 'account' })
  expect(matchRoute('/account/payments')).toEqual({ name: 'payments' })
  expect(matchRoute('/account/payments/')).toEqual({ name: 'payments' })
  for (const path of [
    '/exchanges',
    '/exchanges/not-an-id',
    `/exchanges/${ID}/other`,
    '/i',
    '/en/x',
    '/help/signing/more',
    '/help/Signing',
  ]) {
    expect(matchRoute(path), path).toEqual({ name: 'notFound' })
  }
})

test('an invitation link carries its token in the fragment, never in the path or query', () => {
  const token = 'a3'.repeat(32)
  const link = new URL(paths.invitation('es', token), 'https://app.example')
  expect(link.pathname).toBe('/es/i')
  expect(link.search).toBe('')
  expect(link.hash).toBe(`#${token}`)
  expect(matchRoute(link.pathname)).toEqual({ name: 'invitation', language: 'es' })
  expect(invitationToken(link.hash)).toBe(token)
})

test('a fragment that is not a token is not treated as one', () => {
  for (const hash of ['', '#', '#content', '#short', '#has spaces in it, plainly']) {
    expect(invitationToken(hash), hash).toBeNull()
  }
})
