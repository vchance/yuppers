import type { ExchangeView as Exchange } from '@yuppers/api-client';
import {
  inviteeKind,
  labelText,
  otherPartyName,
  phoneAsTyped,
  type IssuedInvitation,
} from '@yuppers/shared';
import { useEffect, useRef, useState } from 'react';
import { type Text, type View } from 'react-native';

import { InvitationLink } from '../components/InvitationLink';
import { Actions, Button, Heading, Hint, Notice, P, Screen } from '../components/ui';
import { focusOn } from '../lib/accessibility';
import { useI18n } from '../lib/context';

interface Props {
  exchange: Exchange;
  /** The link just issued with the first proposal, shown once. */
  issued: IssuedInvitation;
  /** The person opened a way to send the link. */
  onShared(): void;
  /** They are done here, having opened one. */
  onDone(): void;
  /** They chose to send it later: the exchange's screen reminds them. */
  onLater(): void;
}

/**
 * Sending the invitation, as a step of its own after signing (DESIGN.md
 * §8). Yuppers never sends it: the person who signed has to pass the link
 * on, and until they do the other party has nothing. So this is not a card
 * on the exchange's screen to scroll past but the screen itself, headed
 * with who the link is for, saying the rule plainly, and leading with the
 * way that reaches them (`ShareActions`). Once a way is opened, the step
 * says what happens next and offers the way on. Not sending is a choice,
 * made with "I’ll send it later", which is as plain as the rest and leaves
 * a reminder on the exchange's screen; leaving any other way counts the same.
 */
export function SendInvitation({ exchange, issued, onShared, onDone, onLater }: Props) {
  const { wording, fmt } = useI18n();
  const w = wording.invitationLink;
  const name = otherPartyName(exchange) || wording.party.other;
  const [shared, setShared] = useState(false);
  const heading = useRef<Text>(null);
  const done = useRef<View>(null);

  // A screen reader starts from the heading, and once a way to send the
  // link is opened goes to the way on: what was pressed may have taken the
  // person to another app and back.
  useEffect(() => {
    focusOn(heading.current as unknown as View | null);
  }, []);
  useEffect(() => {
    if (shared) focusOn(done.current);
  }, [shared]);

  function markShared() {
    setShared(true);
    onShared();
  }

  return (
    <Screen key="send">
      {/* Nothing in the name may turn the words around it or break the line. */}
      <Heading headingRef={heading}>{fmt(w.sendTitle, { name: labelText(name) })}</Heading>
      <Notice tone="warning" quiet>
        {fmt(w.sendRule, { name })}
      </Notice>
      <P>
        {/* A US number is written the American way, however it was typed. */}
        {inviteeKind(issued.boundTo) === 'anyone'
          ? w.forAnyoneSummary
          : fmt(w.boundSummary, { identifier: phoneAsTyped(issued.boundTo?.trim() ?? '') })}
      </P>
      <InvitationLink token={issued.token} boundTo={issued.boundTo} onShared={markShared} />
      {shared ? (
        <>
          <Notice>{fmt(w.sharedNotice, { name })}</Notice>
          <Actions>
            <Button
              variant="primary"
              label={w.sendDone}
              onPress={onDone}
              buttonRef={(control) => {
                done.current = control;
              }}
            />
          </Actions>
        </>
      ) : (
        <>
          <Actions>
            <Button variant="link" label={w.later} onPress={onLater} />
          </Actions>
          <Hint>{fmt(w.laterHint, { name })}</Hint>
        </>
      )}
    </Screen>
  );
}
