/*
 * The exchange a fresh start has just made. Its page was opened at `/new/...`
 * with no exchange; once there is one the address moves to
 * `/exchanges/{id}`, and the same page stays up (and the keyboard where it
 * was) instead of loading the exchange again.
 */
export const FRESH = 'fresh'

export const fresh: { id: string | null } = { id: null }
