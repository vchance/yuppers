import type { components } from '@yuppers/api-client';
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
  type Actions as ExchangeActions,
  type Move,
  type Slot,
} from '@yuppers/shared';
import { useState } from 'react';

import { StatusChip } from '../components/StatusChip';
import { Actions, Button, Failure, Hint, Notice, P, Panel, TextField } from '../components/ui';
import { useI18n } from '../lib/context';

type Contribution = components['schemas']['ContributionDto'];
type Status = components['schemas']['Status'];

interface Props {
  contribution: Contribution;
  status: Status;
  /** When it came to stand this way, RFC 3339; the service says. */
  since: string | null;
  you: Slot;
  /** The other party's name, for telling the person who an action affects. */
  otherName: string;
  /** Whether the agreement is still in force; a closed one is only read. */
  active: boolean;
  actions: ExchangeActions;
}

/**
 * Where one contribution stands and what the reader can do about it: the
 * provider marks it delivered, the recipient confirms, disputes or waives
 * (DESIGN.md §5.2). A claim is never a confirmation, and the two are worded
 * differently so neither party mistakes one for the other. Money is paid
 * outside the product and only recorded here, so for money the words are for
 * paying and receiving, never for delivering (DESIGN.md §11).
 */
export function Fulfillment({ contribution, status, since, you, otherName, active, actions }: Props) {
  const { wording, fmt, moment } = useI18n();
  const w = wording.exchange;
  const money = contribution.type === 'MONEY';
  const role = contribution.from === you ? 'PROVIDER' : 'RECIPIENT';
  const moves = active ? movesFor(status, role) : [];
  const panelOf = (move: Move) => movePanel(contribution.id, move);
  const opened = moves.find((move) => actions.panel === panelOf(move));
  // A claim nobody answers stays a claim (DESIGN.md §5.2). After a while the
  // provider is pointed to the way out, rather than left waiting.
  const stuck = active && role === 'PROVIDER' && waitingLong(status, since);

  return (
    <>
      <StatusChip status={status}>{statusWording(wording, status, money)}</StatusChip>
      {stuck && since ? (
        <Notice quiet>
          {fmt(money ? w.waitingLongMoney : w.waitingLong, { name: otherName, date: moment(since) })}
        </Notice>
      ) : null}
      {/* A dispute is recorded, never decided (DESIGN.md §14.1). */}
      {status === 'DISPUTED' && (
        <>
          <P>{wording.dispute.weRecord}</P>
          {active && (
            <>
              <Hint>{wording.dispute.pointer}</Hint>
              <Actions>
                <Button
                  variant="link"
                  label={wording.trouble.open}
                  disabled={actions.busy}
                  onPress={() => actions.open(troublePanel('DISAGREE'))}
                />
              </Actions>
            </>
          )}
        </>
      )}
      {moves.length > 0 && (
        <Actions>
          {moves.map((move) => (
            <Button
              key={move}
              label={moveWording(wording, move, money)}
              expanded={opened === move}
              disabled={actions.busy}
              onPress={() => actions.open(panelOf(move))}
            />
          ))}
        </Actions>
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
  );
}

interface MovePanelProps {
  move: Move;
  money: boolean;
  contribution: string;
  otherName: string;
  actions: ExchangeActions;
}

/** The second look before a move is sent, with the note it takes or needs. */
function MovePanel({ move, money, contribution, otherName, actions }: MovePanelProps) {
  const { wording, fmt } = useI18n();
  const w = wording.exchange;
  const [note, setNote] = useState('');
  const [missing, setMissing] = useState(false);
  const { takes, needs, label } = noteFor(move);
  const title = moveWording(wording, move, money);

  function submit() {
    const command = moveCommand(move, contribution, note);
    if (command) void actions.run(command);
    else setMissing(true);
  }

  return (
    <Panel title={title}>
      <P>{fmt(moveTextWording(wording, move, money), { name: otherName })}</P>
      {move === 'DISPUTE' && (
        <>
          <P>{wording.dispute.weRecord}</P>
          <Hint>{wording.dispute.pointer}</Hint>
        </>
      )}
      {takes && (
        <TextField
          label={w[label]}
          hint={w.noteRecord}
          required={needs}
          error={missing ? w.noteRequired : null}
          multiline
          maxLength={NOTE_MAX_CHARS}
          value={note}
          onChangeText={(next) => {
            setNote(next);
            setMissing(false);
          }}
        />
      )}
      <Failure code={actions.failure} />
      <Actions>
        <Button variant="primary" label={title} disabled={actions.busy} onPress={submit} />
        <Button label={wording.common.cancel} disabled={actions.busy} onPress={actions.close} />
      </Actions>
    </Panel>
  );
}
