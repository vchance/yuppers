import { expect, test } from 'vitest'

import { wordingFor } from './language'
import { formatMessage, type MessageValues } from './message'
import {
  boundToLabel,
  boundToProblem,
  boundToProblemText,
  invitationBoundTo,
  invitationOptions,
  invitationForProblem,
  NAMED_INVITATION,
  shareAddresses,
} from './share'

const LINK = 'https://yuppers.example/en/i#Abc_def-0123456789xyz'
const MESSAGE = `I’ve sent you a yup to review: ${LINK}`

/** The query of a `mailto:`, `sms:` or web address, decoded. */
function query(address: string): URLSearchParams {
  return new URLSearchParams(address.slice(address.indexOf('?') + 1))
}

test('each way of sharing carries the whole link, its token encoded inside the message', () => {
  const found = shareAddresses({ message: MESSAGE, subject: 'You’ve got a yup', boundTo: null })

  expect(found.email.startsWith('mailto:?subject=')).toBe(true)
  expect(query(found.email).get('subject')).toBe('You’ve got a yup')
  expect(query(found.email).get('body')).toBe(MESSAGE)
  expect(found.sms.startsWith('sms:?body=')).toBe(true)
  expect(query(found.sms).get('body')).toBe(MESSAGE)
  const whatsApp = new URL(found.whatsApp)
  expect(whatsApp.origin + whatsApp.pathname).toBe('https://wa.me/')
  expect(whatsApp.searchParams.get('text')).toBe(MESSAGE)

  // The `#` is encoded, so the token is part of the text and not taken for
  // the address's own fragment; spaces are %20, never `+`.
  for (const address of Object.values(found)) {
    expect(address).not.toContain('#')
    expect(address).not.toContain('+')
    expect(address).toContain('%23Abc_def-0123456789xyz')
  }
})

test('an invitation made for someone addresses the message to them', () => {
  const email = shareAddresses({ message: MESSAGE, subject: 'S', boundTo: ' carla@example.test ' })
  expect(email.email.startsWith('mailto:carla@example.test?subject=S&body=')).toBe(true)
  expect(email.sms.startsWith('sms:?body=')).toBe(true)

  const phone = shareAddresses({ message: MESSAGE, subject: 'S', boundTo: '+1 (202) 555-0142' })
  expect(phone.sms.startsWith('sms:+12025550142?body=')).toBe(true)
  expect(phone.email.startsWith('mailto:?subject=')).toBe(true)

  // Something that is neither is left out rather than half used.
  const odd = shareAddresses({ message: MESSAGE, subject: 'S', boundTo: 'carla' })
  expect(odd.email.startsWith('mailto:?')).toBe(true)
  expect(odd.sms.startsWith('sms:?')).toBe(true)
  // An address with characters a mailto must encode keeps its `@`.
  const plus = shareAddresses({ message: MESSAGE, subject: 'S', boundTo: 'carla+yup@example.test' })
  expect(plus.email.startsWith('mailto:carla%2Byup@example.test?')).toBe(true)
})

test('who an invitation is for is checked gently, by the service’s own rule', () => {
  const both = { phone: true, countryCodes: ['+1'], codeSender: null }
  const emailOnly = { phone: false, countryCodes: [], codeSender: null }

  // Empty names nobody, which is fine.
  expect(boundToProblem('  ', emailOnly)).toBeNull()
  for (const fine of ['carla@example.test', 'Carla.Diaz@mail.example.co']) {
    expect(boundToProblem(fine, both)).toBeNull()
    expect(boundToProblem(fine, emailOnly)).toBeNull()
  }
  expect(boundToProblem('+1 202 555 0142', both)).toBeNull()

  for (const malformed of [
    'carla@',
    '@example.test',
    'carla@example',
    'carla@example..test',
    'car la@example.test',
    'cárla@example.test',
  ]) {
    expect(boundToProblem(malformed, both)).toBe('invalid')
    expect(boundToProblem(malformed, emailOnly)).toBe('invalidEmail')
  }
  for (const malformed of ['carla', '2025550142', '+0123456789', '+12345', '+1 202 555 0142 ext 3']) {
    expect(boundToProblem(malformed, both)).toBe('invalid')
  }
  // A phone number where codes go by email only: they could never sign in with it.
  expect(boundToProblem('+1 202 555 0142', emailOnly)).toBe('emailOnly')
  expect(boundToProblem('carla', emailOnly)).toBe('invalidEmail')
  // Nor with a number of a country that is not texted.
  expect(boundToProblem('+44 20 7946 0958', both)).toBe('country')
  // While the service has not said, only the shape.
  expect(boundToProblem('+44 20 7946 0958', null)).toBeNull()
  expect(boundToProblem('carla@', null)).toBe('invalid')
})

test('the field asks for an email address until phone numbers are offered, and says what is wrong', () => {
  const w = wordingFor('en').invitationLink
  const fmt = (message: string, values: MessageValues) => formatMessage(message, values, 'en')
  const both = { phone: true, countryCodes: ['+1', '+52'], codeSender: null }
  expect(boundToLabel(w, null)).toBe(w.forLabelEmail)
  expect(boundToLabel(w, { ...both, phone: false })).toBe(w.forLabelEmail)
  expect(boundToLabel(w, both)).toBe(w.forLabel)
  expect(boundToProblemText('invalid', w, both, fmt)).toBe(w.forInvalid)
  expect(boundToProblemText('invalidEmail', w, both, fmt)).toBe(w.forInvalidEmail)
  expect(boundToProblemText('emailOnly', w, both, fmt)).toBe(w.forEmailOnly)
  expect(boundToProblemText('country', w, both, fmt)).toBe(
    'We can only send sign-in codes to phone numbers starting +1, +52, so they couldn’t use the link with this number.',
  )
})

test('someone has to be named, unless a link for anyone is chosen on purpose', () => {
  const w = wordingFor('en').invitationLink
  const fmt = (message: string, values: MessageValues) => formatMessage(message, values, 'en')
  const both = { phone: true, countryCodes: ['+1'], codeSender: null }
  const emailOnly = { ...both, phone: false }

  // Naming them is where it starts, and an empty field is not a choice.
  expect(NAMED_INVITATION).toEqual({ anyone: false, to: '' })
  expect(invitationForProblem(NAMED_INVITATION, both)).toBe('missing')
  expect(invitationForProblem({ anyone: false, to: '   ' }, both)).toBe('missing')
  expect(boundToProblemText('missing', w, both, fmt)).toBe(w.forMissing)
  expect(boundToProblemText('missing', w, emailOnly, fmt)).toBe(w.forMissingEmail)
  expect(boundToProblemText('missing', w, null, fmt)).toBe(w.forMissingEmail)

  // Named, it is checked as before.
  expect(invitationForProblem({ anyone: false, to: 'carla@' }, both)).toBe('invalid')
  expect(invitationForProblem({ anyone: false, to: '+44 20 7946 0958' }, both)).toBe('country')
  expect(invitationForProblem({ anyone: false, to: ' carla@example.test ' }, both)).toBeNull()
  expect(invitationBoundTo({ anyone: false, to: ' carla@example.test ' })).toBe(
    'carla@example.test',
  )

  // A link for anyone names nobody, whatever was typed before choosing it.
  const anyone = { anyone: true, to: 'carla@' }
  expect(invitationForProblem(anyone, both)).toBeNull()
  expect(invitationBoundTo(anyone)).toBeNull()
})

test('the service is told outright who a link is for: the person named, or anyone', () => {
  expect(invitationOptions(' carla@example.test ')).toEqual({ bound_to: 'carla@example.test' })
  expect(invitationOptions(null)).toEqual({ for_anyone: true })
  expect(invitationOptions('  ')).toEqual({ for_anyone: true })
})
