import type { components, ExchangeView as Exchange } from '@yuppers/api-client'
import {
  moveCommand,
  movePanel,
  movesFor,
  moveTextWording,
  moveWording,
  noteFor,
  NOTE_MAX_CHARS,
  payOffered,
  statusWording,
  troublePanel,
  waitingLong,
  type Move,
} from '@yuppers/shared'
import { lazy, Suspense, useId, useRef, useState, type FormEvent } from 'react'

import { useI18n } from '../app/context'
import { Panel } from '../components/Panel'
import { StatusChip } from '../components/StatusChip'
import { Failure, Field } from '../components/ui'
import type { Actions } from '../lib/actions'
import type { Slot } from '../lib/api'

// Only a payer whose payee shows payment options ever opens it.
const PaySheet = lazy(() =>
  import('../components/PaySheet').then((module) => ({ default: module.PaySheet })),
)

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
  /**
   * The exchange, where the reader may be offered "Pay" (`payOffered`): it
   * carries the other party's payment options, when they show them.
   */
  exchange?: Exchange
  /** Brings the exchange up to date, once the pay sheet has read it again. */
  onChange?(exchange: Exchange): void
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
  exchange,
  onChange,
}: Props) {
  const { wording, fmt, moment, money: formatMoney } = useI18n()
  const w = wording.exchange
  const money = contribution.type === 'MONEY'
  const role = contribution.from === you ? 'PROVIDER' : 'RECIPIENT'
  const moves = active ? movesFor(status, role) : []
  const panelOf = (move: Move) => movePanel(contribution.id, move)
  const opened = moves.find((move) => actions.panel === panelOf(move))
  // A claim nobody answers stays a claim (DESIGN.md §5.2). After a while the
  // provider is pointed to the way out, rather than left waiting.
  const stuck = active && role === 'PROVIDER' && waitingLong(status, since)
  // Paying in another app, where the payee shows their payment options. It
  // records nothing: "I've paid" stays the payer's own move.
  const pay = exchange && active && payOffered(exchange, contribution, status)
  const [paying, setPaying] = useState(false)
  const payButton = useRef<HTMLButtonElement>(null)
  const claim: Move = status === 'DISPUTED' ? 'RECLAIM' : 'CLAIM'

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
      {pay && exchange && (
        <div className="actions">
          <button
            type="button"
            className="primary"
            ref={payButton}
            aria-haspopup="dialog"
            disabled={actions.busy}
            onClick={() => setPaying(true)}
          >
            {fmt(wording.payments.payButton, {
              name: otherName,
              amount: formatMoney(contribution.amount_minor ?? 0, exchange.currency),
            })}
          </button>
        </div>
      )}
      {paying && exchange && (
        <Suspense fallback={null}>
          <PaySheet
            exchange={exchange}
            contribution={contribution}
            otherName={otherName}
            onClose={(found) => {
              setPaying(false)
              if (found) onChange?.(found)
              payButton.current?.focus()
            }}
            onPaid={(found) => {
              setPaying(false)
              if (found) onChange?.(found)
              // The claim the payer sends themselves, as without the sheet.
              actions.open(panelOf(claim))
            }}
          />
        </Suspense>
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
