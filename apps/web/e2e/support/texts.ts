import { readFileSync } from 'node:fs'

/*
 * Text messages, read back from a log. With SMS_DELIVERY=log the service
 * writes each text it would have sent, the number masked but for its last
 * two digits:
 *
 *   ... text message (development delivery) to="+1••••••••23" text="123456 is your Yuppers sign-in code. ..."
 */

/** A US number nobody else uses, as the service stores it. */
export function number(): string {
  const digits = () => Math.floor(Math.random() * 10)
  return `+1${7 + (digits() % 3)}${digits()}${digits()}${2 + (digits() % 8)}${Array.from({ length: 6 }, digits).join('')}`
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
