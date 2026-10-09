import type { ExchangeView } from '@yuppers/api-client';
import {
  useInvitationAddress,
  type BoundAddress,
  type InvitationAddressApi,
} from '@yuppers/shared';

import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { CombineOffer } from './CombineOffer';
import { ConsentCheckbox } from './ConsentCheckbox';
import { Actions, Button, Card, Failure, Heading, Hint, P, TextField } from './ui';

interface Props {
  token: string;
  sentTo: BoundAddress;
  onOpened(exchange: ExchangeView): void;
  onSignOut(): void;
  /** For tests: the service's calls. */
  client?: InvitationAddressApi;
}

/**
 * An invitation sent to an address the account does not have, as on the
 * web (`apps/web/src/components/InvitationAddress.tsx`): shown masked, with
 * a code sent there to add it; the offer to combine if it is on another of
 * the person's accounts; and signing in with it instead.
 */
export function InvitationAddress({ token, sentTo, onOpened, onSignOut, client = api }: Props) {
  const { wording, fmt, language } = useI18n();
  const { setAccount } = useSession();
  const w = wording.invitation;
  const control = useInvitationAddress(client, token, sentTo, language, onOpened, setAccount);
  const identifier = sentTo.masked;
  const consent = control.codeConsent;

  if (control.offer) {
    return (
      <CombineOffer
        offer={control.offer}
        busy={control.busy}
        failure={control.failure}
        onCombine={() => void control.combine()}
        onCancel={control.dismissOffer}
      />
    );
  }

  return (
    <Card>
      <Heading level={2}>{fmt(w.sentTo, { identifier })}</Heading>
      {sentTo.replaces ? (
        <P>{sentTo.kind === 'PHONE' ? w.sentToReplacesPhone : w.sentToReplacesEmail}</P>
      ) : null}
      {control.step === 'offer' ? (
        <>
          {consent.shown ? (
            <ConsentCheckbox
              wording={wording.smsCode.verifyNumber}
              checked={consent.checked}
              onChange={consent.setChecked}
              disabled={control.busy}
            />
          ) : null}
          {consent.missing ? <Hint>{wording.smsCode.tickToSend}</Hint> : null}
          <Failure code={control.failure} />
          <Actions>
            <Button
              testID="send-address-code"
              label={fmt(w.sendAddressCode, { identifier })}
              variant="primary"
              disabled={control.busy || consent.missing}
              hint={consent.missing ? wording.smsCode.tickToSend : undefined}
              onPress={() => void control.sendCode()}
            />
          </Actions>
        </>
      ) : (
        <>
          <P>{fmt(w.addressCodeSent, { identifier })}</P>
          <TextField
            label={wording.signIn.codeLabel}
            hint={wording.signIn.codeHint}
            required
            keyboardType="number-pad"
            autoComplete="one-time-code"
            textContentType="oneTimeCode"
            maxLength={6}
            value={control.code}
            onChangeText={control.setCode}
          />
          <Failure code={control.failure} />
          <Actions>
            <Button
              label={sentTo.replaces ? w.replaceAndOpen : w.addAndOpen}
              variant="primary"
              disabled={control.busy}
              onPress={() => void control.confirm()}
            />
            <Button
              label={wording.signIn.resend}
              disabled={control.busy}
              onPress={() => void control.sendCode()}
            />
          </Actions>
        </>
      )}
      <P>{fmt(w.signInInstead, { identifier })}</P>
      <Actions>
        <Button label={wording.nav.signOut} variant="link" onPress={onSignOut} />
      </Actions>
    </Card>
  );
}
