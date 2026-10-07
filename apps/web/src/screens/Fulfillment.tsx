import type { components } from '@yuppers/api-client'
import {
  moveCommand,
  movePanel,
  movesFor,
  moveTextWording,
  moveWording,
  noteFor,
  NOTE_MAX_CHARS,
  statusWording,
  troublePanel,
  waitingLong,
  type Move,
} from '@yuppers/shared'
import { useId, useState, type FormEvent } from 'react'

import { useI18n } from '../app/context'
import { Panel } from '../components/Panel'
import { StatusChip } from '../components/StatusChip'
import { Failure, Field } from '../components/ui'
import type { Actions } from '../lib/actions'
import type { Slot } from '../lib/api'

type Contribution = components['schemas']['ContributionDto']
type Status = components['schemas']['Status']

interface Props {
  contribution: Contribution
  status: Status
  /** When it came to stand this way, RFC 3339; the service says. */
  since: string | null
  you: Slot
  /** The other party's name, for telling the person who an action affects. */
  otherName: string
  /** Whether the agreement is still in force; a closed one is only read. */
  active: boolean
  actions: Actions
}

/**
 * Where one contribution stands and what the reader can do about it: the
 * provider marks it delivered, the recipient confirms, disputes or waives
 * (DESIGN.md §5.2). A claim is never a confirmation, and the two are worded
 * differently so neither party mistakes one for the other. Money is paid
 * outside the product and only recorded here, so for money the words are for
 * paying and receiving, never for delivering (DESIGN.md §11).
 */
export function Fulfillment({
  contribution,
  status,
  since,
  you,
  otherName,
  active,
  actions,
}: Props) {
  const { wording, fmt, moment } = useI18n()
  const w = wording.exchange
  const money = contribution.type === 'MONEY'
  const role = contribution.from === you ? 'PROVIDER' : 'RECIPIENT'
  const moves = active ? movesFor(status, role) : []
  const panelOf = (move: Move) => movePanel(contribution.id, move)
  const opened = moves.find((move) => actions.panel === panelOf(move))
  // A claim nobody answers stays a claim (DESIGN.md §5.2). After a while the
  // provider is pointed to the way out, rather than left waiting.
  const stuck = active && role === 'PROVIDER' && waitingLong(status, since)

  return (
    <>
      <p className="status">
        <StatusChip status={status}>{statusWording(wording, status, money)}</StatusChip>
      </p>
      {stuck && since && (
        <p className="notice">
          {fmt(money ? w.waitingLongMoney : w.waitingLong, {
            name: otherName,
            date: moment(since),
          })}
        </p>
      )}
      {/* A dispute is recorded, never decided (DESIGN.md §14.1). */}
      {status === 'DISPUTED' && (
        <div className="dispute-note">
          <p>{wording.dispute.weRecord}</p>
          {active && (
            <>
              <p className="hint">{wording.dispute.pointer}</p>
              <div className="actions">
                <button
                  type="button"
                  className="link"
                  disabled={actions.busy}
                  onClick={() => actions.open(troublePanel('DISAGREE'))}
                >
                  {wording.trouble.open}
                </button>
              </div>
            </>
          )}
        </div>
      )}
      {moves.length > 0 && (
        <div className="actions">
          {moves.map((move) => (
            <button
              key={move}
              type="button"
              aria-expanded={opened === move}
              disabled={actions.busy}
              onClick={() => actions.open(panelOf(move))}
            >
              {moveWording(wording, move, money)}
            </button>
          ))}
        </div>
      )}
      {opened && (
        <MovePanel
          key={opened}
          move={opened}
          money={money}
          contribution={contribution.id}
          otherName={otherName}
          actions={actions}
        />
      )}
    </>
  )
}

interface MovePanelProps {
  move: Move
  money: boolean
  contribution: string
  otherName: string
  actions: Actions
}

function MovePanel({ move, money, contribution, otherName, actions }: MovePanelProps) {
  const { wording, fmt } = useI18n()
  const w = wording.exchange
  const [note, setNote] = useState('')
  const [missing, setMissing] = useState(false)

  const { takes: takesNote, needs, label } = noteFor(move)
  const title = moveWording(wording, move, money)
  const noteId = useId()

  function submit(event: FormEvent) {
    event.preventDefault()
    const command = moveCommand(move, contribution, note)
    if (command) void actions.run(command)
    else {
      setMissing(true)
      // Back to the note, whose error is read with it.
      document.getElementById(noteId)?.focus()
    }
  }

  return (
    <Panel title={title}>
      <form noValidate onSubmit={submit}>
        <p>{fmt(moveTextWording(wording, move, money), { name: otherName })}</p>
        {move === 'DISPUTE' && (
          <>
            <p>{wording.dispute.weRecord}</p>
            <p className="hint">{wording.dispute.pointer}</p>
          </>
        )}
        {takesNote && (
          <Field
            label={w[label]}
            hint={w.noteRecord}
            id={noteId}
            required={needs}
            error={missing ? w.noteRequired : null}
          >
            {(control) => (
              <textarea
                {...control}
                rows={3}
                maxLength={NOTE_MAX_CHARS}
                value={note}
                onChange={(event) => {
                  setNote(event.target.value)
                  setMissing(false)
                }}
              />
            )}
          </Field>
        )}
        <Failure code={actions.failure} />
        <div className="actions">
          <button type="submit" className="primary" disabled={actions.busy}>
            {title}
          </button>
          <button type="button" disabled={actions.busy} onClick={actions.close}>
            {wording.common.cancel}
          </button>
        </div>
      </form>
    </Panel>
  )
}
