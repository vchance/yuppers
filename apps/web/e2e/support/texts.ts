import { readFileSync } from 'node:fs'

/*
 * Text messages, and codes for phone numbers, read back from a log. With
 * SMS_DELIVERY=log the service writes each agreement update it would have
 * sent, the number masked but for its last two digits:
 *
 *   ... text message (development delivery) to="+1••••••••23" text="Yuppers.app: an agreement you turned on updates for has changed. ..."
 */

/**
 * Area codes of the United States only: +1 also covers Canada and the
 * Caribbean, whose numbers the service does not text (942, say, is
 * Toronto's), so a random area code would fail now and then.
 */
const US_AREA_CODES = ['212', '305', '312', '415', '503', '617', '702', '713', '808', '917']

/** A US number nobody else uses, as the service stores it. */
export function number(): string {
  const digits = () => Math.floor(Math.random() * 10)
  const area = US_AREA_CODES[Math.floor(Math.random() * US_AREA_CODES.length)]
  return `+1${area}${2 + (digits() % 8)}${Array.from({ length: 6 }, digits).join('')}`
}

/** The texts a log shows were sent to `phone`, oldest first. */
export function textsTo(phone: string, log: string): string[] {
  let text: string
  try {
    text = readFileSync(log, 'utf8')
  } catch {
    return []
  }
  const masked = `+1••••••••${phone.slice(-2)}`
  return text
    .split('\n')
    .filter((line) => line.includes('text message (development delivery)') && line.includes(masked))
    .map((line) => /text="((?:[^"\\]|\\.)*)"/.exec(line)?.[1]?.replaceAll('\\"', '"') ?? '')
}

/**
 * The one-time codes a log shows were made for `phone`, oldest first. Twilio
 * Verify texts codes in production; with SMS_CODE_DELIVERY=log the service
 * makes each itself and writes it to the log instead, the number masked:
 *
 *   ... one-time code (development delivery) to="+1••••••••23" code="123456" purpose="sign-in" language="en"
 */
export function codesTo(phone: string, log: string): string[] {
  let text: string
  try {
    text = readFileSync(log, 'utf8')
  } catch {
    return []
  }
  const masked = `+1••••••••${phone.slice(-2)}`
  return text
    .split('\n')
    .filter((line) => line.includes('one-time code (development delivery)') && line.includes(masked))
    .map((line) => /code="?(\d{6})/.exec(line)?.[1])
    .filter((code): code is string => code !== undefined)
}

/** Waits until `find` finds something, and returns it. */
export async function waitFor<T>(find: () => T | undefined, what: string): Promise<T> {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const found = find()
    if (found !== undefined) return found
    await new Promise((settle) => setTimeout(settle, 200))
  }
  throw new Error(`timed out waiting for ${what}`)
}
