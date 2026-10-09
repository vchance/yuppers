import type { components, ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client';
import {
  leaveExchange,
  verificationText,
  type Actions as ExchangeActions,
} from '@yuppers/shared';
import { useRouter } from 'expo-router';
import { useState } from 'react';

import { Actions, Button, Card, Failure, Heading, Hint, Notice, P, Panel } from '../components/ui';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';

/*
 * Someone opened an invitation that named nobody, and the initiator has not
 * yet said whether it is the person they invited (DESIGN.md §8). The
 * initiator confirms them or removes them; until then the claimant can sign
 * or leave, and nothing else.
 */

const REJECT = 'claimant-reject';
const LEAVE = 'claimant-leave';

interface ConfirmProps {
  exchange: Exchange;
  claimant: components['schemas']['Claimant'];
  actions: ExchangeActions;
  /** The claimant has been removed, and the place is free again. */
  onRejected(): void;
}

/**
 * The initiator's question: is this who you invited? Yes confirms them. No
 * removes them, which is said in full before it is done, because it cannot
 * be undone and voids anything they signed.
 */
export function ConfirmClaimant({ exchange, claimant, actions, onRejected }: ConfirmProps) {
  const { wording, fmt } = useI18n();
  const w = wording.exchange;
  const c = wording.claimant;
  const signed = exchange.open_revision?.accepted_by.includes('B') ?? false;

  async function reject() {
    if (await actions.run({ type: 'REJECT_COUNTERPARTY' })) onRejected();
  }

  return (
    <Card>
      <Heading level={2}>{w.claimedHeading}</Heading>
      <P>{fmt(w.claimedBody, { name: claimant.display_name, identifier: claimant.identifier })}</P>
      {signed && <P>{w.claimedSigned}</P>}
      <Hint>{w.notThem}</Hint>
      <Actions>
        <Button
          variant="primary"
          label={w.confirmCounterparty}
          disabled={actions.busy}
          onPress={() => void actions.run({ type: 'CONFIRM_COUNTERPARTY' })}
        />
        <Button
          label={c.reject}
          expanded={actions.panel === REJECT}
          disabled={actions.busy}
          onPress={() => actions.open(REJECT)}
        />
      </Actions>

      {actions.panel === REJECT && (
        <Panel title={c.rejectTitle}>
          <P>{fmt(c.rejectRemoves, { name: claimant.display_name })}</P>
          <P>{c.rejectVoids}</P>
          <P>{c.rejectKeeps}</P>
          <P>{c.rejectQuiet}</P>
          <Failure code={actions.failure} />
          <Actions>
            <Button
              variant="primary"
              label={c.confirmReject}
              disabled={actions.busy}
              onPress={() => void reject()}
            />
            <Button label={wording.common.cancel} disabled={actions.busy} onPress={actions.close} />
          </Actions>
        </Panel>
      )}
    </Card>
  );
}

interface WaitingProps {
  exchange: Exchange;
  /** The initiator, as the proposal names them. */
  otherName: string;
  actions: ExchangeActions;
}

/**
 * What a claimant is told while the initiator has not confirmed them: that
 * they can sign, that they cannot yet decline or propose changes, and that
 * they can leave. Leaving is their only way out, so it is always offered.
 */
export function ClaimantWaiting({ exchange, otherName, actions }: WaitingProps) {
  const { wording, fmt } = useI18n();
  const router = useRouter();
  const w = wording.exchange;
  const c = wording.claimant;
  const signed = exchange.open_revision?.accepted_by.includes('B') ?? false;
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<ErrorCode | null>(null);

  async function leave() {
    setBusy(true);
    setFailure(null);
    const refused = await leaveExchange(api, exchange.id);
    // The exchange no longer exists for this person; there is nothing here
    // to come back to.
    if (refused === null) router.dismissTo('/');
    else {
      setFailure(refused);
      setBusy(false);
    }
  }

  return (
    <>
      <Notice quiet>
        <P>
          {fmt(signed ? w.waitingConfirmationSigned : w.waitingConfirmation, { name: otherName })}
        </P>
        <P>{c.limits}</P>
        <Actions>
          <Button
            label={c.leave}
            expanded={actions.panel === LEAVE}
            disabled={busy || actions.busy}
            onPress={() => {
              setFailure(null);
              actions.open(LEAVE);
            }}
          />
        </Actions>
      </Notice>

      {actions.panel === LEAVE && (
        <Panel title={c.leave}>
          <P>{fmt(c.leaveText, { name: otherName })}</P>
          {signed && <P>{c.leaveVoids}</P>}
          <Failure code={failure} />
          <Actions>
            <Button
              variant="primary"
              label={c.confirmLeave}
              disabled={busy}
              onPress={() => void leave()}
            />
            <Button label={wording.common.cancel} disabled={busy} onPress={actions.close} />
          </Actions>
        </Panel>
      )}
    </>
  );
}

interface VoidSignatureProps {
  signature: components['schemas']['VoidSignature'];
  when(instant: string): string;
}

/**
 * In the record: a signature left by someone who opened the invitation and
 * was removed, or left, before being confirmed. It is kept because it
 * happened; it names nobody and counts for nothing.
 */
export function VoidSignature({ signature, when }: VoidSignatureProps) {
  const { wording, fmt } = useI18n();
  const w = wording.record;
  return (
    <>
      <P>
        {fmt(wording.claimant.voidSignature, {
          date: when(signature.signed_at),
          since: when(signature.void_since),
        })}
      </P>
      <Hint>
        {fmt(w.verifiedBy, {
          method: verificationText(signature.verification, w.export.verification),
        })}
      </Hint>
      <Hint>{fmt(w.verifiedAt, { date: when(signature.verification.verified_at) })}</Hint>
      <Hint>
        {fmt(w.consentShown, {
          version: signature.consent.version,
          language: signature.consent.language,
        })}
      </Hint>
    </>
  );
}

/**
 * On the list of blocked people: an exchange the reader has left still names
 * whom they blocked, and is not theirs to open.
 */
export function LeftExchangeNote() {
  const { wording } = useI18n();
  return <Hint>{wording.claimant.blockedAfterLeaving}</Hint>;
}
