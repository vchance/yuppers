import type { components } from '@yuppers/api-client'

type Status = components['schemas']['Status']

/**
 * Where a contribution stands, in words, on a chip whose fill and icon say
 * it as well: a ring for not yet, a half-filled ring for waiting to be
 * confirmed, an exclamation for disputed, a check for confirmed, a dash for
 * waived or removed. So no status is told from another by colour alone.
 */
export function StatusChip({ status, children }: { status: Status; children: string }) {
  return (
    <span className={`chip chip-${status.toLowerCase()}`}>
      <StatusIcon status={status} />
      {children}
    </span>
  )
}

function StatusIcon({ status }: { status: Status }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      {status === 'PENDING' && <circle cx="8" cy="8" r="6" />}
      {status === 'CLAIMED' && (
        <>
          <circle cx="8" cy="8" r="6" />
          <path className="filled" d="M8 2a6 6 0 0 1 0 12z" />
        </>
      )}
      {status === 'DISPUTED' && <path d="M8 3.5v6M8 12.5v.1" />}
      {status === 'ACCEPTED' && <path d="M3.5 8.5l3 3 6-7" />}
      {(status === 'WAIVED' || status === 'REMOVED') && <path d="M3.5 8h9" />}
    </svg>
  )
}
