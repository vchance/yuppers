import type { components } from '@yuppers/api-client'
import { eventMessage, noteKind, type RecordEvent } from '@yuppers/shared'

import { useI18n } from '../app/context'
import './history.css'
import { WithName, Written } from './ui'

type Schemas = components['schemas']

interface Props {
  /** Oldest first. */
  events: readonly RecordEvent[]
  /** Each party's name as the agreement writes it. */
  parties: Schemas['Parties']
  /** The party reading, who is spoken to as "you". `null` names everyone. */
  reader: Schemas['Slot'] | null
  /** How a moment in time is written here. */
  when(instant: string): string
  /** The ids of the money contributions, which are spoken of in words for paying and receiving. */
  money?: ReadonlySet<string>
}

/**
 * What happened in an exchange, in order. Each entry is one sentence of the
 * product's, saying who did what, followed by anything the parties wrote
 * with it: the contribution it is about, as they described it, and their
 * message, note, reason or statement. Those are shown exactly as written and
 * set apart as theirs (DESIGN.md §4.2).
 */
export function EventList({ events, parties, reader, when, money }: Props) {
  const { wording, fmt } = useI18n()
  const w = wording.record

  return (
    <ol className="history">
      {events.map((event) => {
        const { message, values } = eventMessage(event, w.events, reader, parties, money)
        return (
          <li key={event.sequence} className={`history-entry ${sideOf(event, reader)}`}>
            <p className="hint">
              <time dateTime={event.at}>{when(event.at)}</time>
            </p>
            <p>
              {typeof values.name === 'string' ? (
                <WithName message={message} name={values.name} values={values} />
              ) : (
                fmt(message, values)
              )}
            </p>
            {event.contribution?.description && (
              <Written>{event.contribution.description}</Written>
            )}
            {event.note && (
              <>
                <p className="label">{w.noteLabels[noteKind(event)]}</p>
                <Written>{event.note}</Written>
              </>
            )}
          </li>
        )
      })}
    </ol>
  )
}

/**
 * Where an entry sits: what the reader did, what the other party did, or
 * what happened to both (and every entry, where nobody reads as "you").
 * Something done from the invited party's place by someone since removed
 * from it was done by neither party as they are now.
 */
function sideOf(event: RecordEvent, reader: Schemas['Slot'] | null): string {
  if (reader === null || event.actor === 'SYSTEM' || event.by_removed_claimant) {
    return 'history-system'
  }
  return event.actor === reader ? 'history-you' : 'history-them'
}
