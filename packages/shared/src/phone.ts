/*
 * Phone numbers as people in the US write them. The service texts US
 * numbers only, so a number is read the way it is written there: ten
 * digits, with or without a leading 1 or +1, and any spaces, dashes, dots or
 * brackets, as in `(856) 548-8780`. What is kept, compared and sent is E.164,
 * `+18565488780`, so a number reaches the same account and the same blind
 * index however it was typed (`backend/src/domain/identity.rs` reads it the
 * same way). A number given with another country code, `+44 …`, is read as
 * that country's, for the service to refuse as not served.
 *
 * The area code and the exchange (the three digits after it) are checked
 * against the North American plan: neither starts with 0 or 1. Which area
 * codes are the United States' is the service's to say (`backend/src/nanp.rs`).
 */

/** What a typed phone number reads as. */
export type PhoneReading =
  /** A +1 number: `+1` and ten digits. */
  | { kind: 'nanp'; phone: string }
  /** A number given with another country code: `+` and 7 to 15 digits. */
  | { kind: 'international'; phone: string }
  | { kind: 'invalid' }

/** Ten digits, area code first, neither it nor the exchange starting with 0 or 1. */
const NATIONAL = /^[2-9]\d{2}[2-9]\d{6}$/

/** Digits and the signs a number is written with, a `+` only first. */
const WRITTEN = /^\+?[\d\s().-]+$/

/** Reads what someone typed as a phone number (see the top of this file). */
export function readPhone(input: string): PhoneReading {
  const typed = input.trim()
  if (!WRITTEN.test(typed)) return { kind: 'invalid' }
  const digits = typed.replace(/\D/g, '')
  let national: string
  if (typed.startsWith('+')) {
    if (!digits.startsWith('1')) {
      const plausible = digits.length >= 7 && digits.length <= 15 && !digits.startsWith('0')
      return plausible ? { kind: 'international', phone: `+${digits}` } : { kind: 'invalid' }
    }
    national = digits.slice(1)
  } else {
    national = digits.length === 11 && digits.startsWith('1') ? digits.slice(1) : digits
  }
  return NATIONAL.test(national) ? { kind: 'nanp', phone: `+1${national}` } : { kind: 'invalid' }
}

/**
 * A US number as the service takes it, `+15551234567`, from what someone
 * typed, or `null` for anything else: another country code, the wrong
 * number of digits, an area code or exchange starting with 0 or 1.
 */
export function usPhone(input: string): string | null {
  const reading = readPhone(input)
  return reading.kind === 'nanp' ? reading.phone : null
}

/**
 * Why a typed phone number would not do, if it would not: `invalid` when it
 * is not one, `country` when its country code is not one of `countryCodes`
 * (such as `+1`). With no country codes, as before the service has said,
 * a number of another country is left for the service to decide.
 */
export function phoneProblem(
  input: string,
  countryCodes: readonly string[],
): 'invalid' | 'country' | null {
  const reading = readPhone(input)
  if (reading.kind === 'invalid') return 'invalid'
  const { phone } = reading
  if (countryCodes.length > 0 && !countryCodes.some((code) => phone.startsWith(code))) {
    return 'country'
  }
  return null
}

/**
 * An email address or phone number as the service is sent it: an address
 * trimmed, a phone number in E.164, and anything else trimmed, for the
 * service to refuse in its own words.
 */
export function identifierToSend(input: string): string {
  const typed = input.trim()
  if (typed.includes('@')) return typed
  const reading = readPhone(typed)
  return reading.kind === 'invalid' ? typed : reading.phone
}

/**
 * A phone number as the screens show it: a +1 number the American way,
 * `(856) 548-8780`; anything else, an email address included, as it is.
 */
export function formatPhone(value: string): string {
  const match = /^\+1(\d{3})(\d{3})(\d{4})$/.exec(value)
  return match ? `(${match[1]}) ${match[2]}-${match[3]}` : value
}

/**
 * What a phone field shows once the person leaves it: a +1 number written
 * the American way, `(856) 548-8780`; anything else as it was typed, to be
 * corrected. An email address is left alone.
 */
export function phoneAsTyped(input: string): string {
  if (input.includes('@')) return input
  const reading = readPhone(input)
  return reading.kind === 'nanp' ? formatPhone(reading.phone) : input
}

/**
 * A phone number as the screens show it with all but its last four digits
 * hidden: `(•••) •••-4567` for a +1 number.
 */
export function maskPhone(phone: string): string {
  const digits = phone.replace(/\D/g, '')
  const last = digits.slice(-4)
  if (phone.startsWith('+1') && digits.length === 11) return `(•••) •••-${last}`
  return `•••${last}`
}

/** The example number fields show, in the format they are read in. */
export const PHONE_EXAMPLE = '(555) 123-4567'
