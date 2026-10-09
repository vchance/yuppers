import type { ExchangeView } from '@yuppers/api-client';
import {
  PHONE_EXAMPLE,
  phoneAsTyped,
  shownIdentifier,
  useInvitationAddress,
  type BoundAddress,
  type InvitationAddressApi,
} from '@yuppers/shared';
import type { ReactNode } from 'react';

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
 * web (`apps/web/src/components/InvitationAddress.tsx`): only its kind is
 * said; the person types the address, which gets a code if it is the one;
 * a code to one of the account's own first where adding it replaces one;
 * the offer to combine if it is on another of the person's accounts; and
 * signing in with it instead.
 */
export function InvitationAddress({ token, sentTo, onOpened, onSignOut, client = api }: Props) {
  const { wording, fmt, language } = useI18n();
  const { account, setAccount } = useSession();
  const w = wording.invitation;
  const own = { email: account?.email ?? null, phone: account?.phone ?? null };
  const control = useInvitationAddress(client, token, sentTo, own, language, onOpened, setAccount);
  const consent = control.codeConsent;
  const phone = sentTo.kind === 'PHONE';

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

  const consentBox = (
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
    </>
  );

  const codeStep = (sentToText: string, label: string) => (
    <>
      <P>{fmt(w.addressCodeSent, { identifier: sentToText })}</P>
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
          label={label}
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
  );

  let body: ReactNode;
  if (control.step === 'prove' && control.proveTo) {
    const to = shownIdentifier(control.proveTo);
    const alternative = control.proveAlternative;
    body = (
      <>
        <P>{fmt(wording.identifiers.proveIntro, { identifier: to })}</P>
        {consentBox}
        <Failure code={control.failure} />
        <Actions>
          <Button
            testID="send-proof-code"
            label={fmt(wording.identifiers.proveSend, { identifier: to })}
            variant="primary"
            disabled={control.busy || consent.missing}
            hint={consent.missing ? wording.smsCode.tickToSend : undefined}
            onPress={() => void control.sendCode()}
          />
          {alternative ? (
            <Button
              label={fmt(wording.identifiers.proveOther, {
                identifier: shownIdentifier(alternative),
              })}
              disabled={control.busy}
              onPress={control.proveElsewhere}
            />
          ) : null}
        </Actions>
      </>
    );
  } else if (control.step === 'proveCode' && control.proveTo) {
    body = codeStep(shownIdentifier(control.proveTo), wording.identifiers.proveConfirm);
  } else if (control.step === 'code' && control.typed) {
    body = codeStep(
      shownIdentifier(control.typed),
      sentTo.replaces ? w.replaceAndOpen : w.addAndOpen,
    );
  } else {
    body = (
      <>
        {phone ? (
          <TextField
            label={wording.identifiers.newPhoneLabel}
            hint={wording.smsUpdates.phoneHint}
            required
            error={control.invalidPhone ? wording.smsUpdates.phoneInvalid : null}
            keyboardType="phone-pad"
            autoComplete="tel-national"
            textContentType="telephoneNumber"
            placeholder={PHONE_EXAMPLE}
            value={control.input}
            onChangeText={control.setInput}
            onBlur={() => control.setInput(phoneAsTyped(control.input))}
          />
        ) : (
          <TextField
            label={wording.identifiers.newEmailLabel}
            required
            keyboardType="email-address"
            autoComplete="email"
            textContentType="emailAddress"
            autoCapitalize="none"
            value={control.input}
            onChangeText={control.setInput}
          />
        )}
        {consentBox}
        <Failure code={control.failure} />
        <Actions>
          <Button
            testID="send-address-code"
            label={wording.identifiers.sendCode}
            variant="primary"
            disabled={control.busy || consent.missing}
            hint={consent.missing ? wording.smsCode.tickToSend : undefined}
            onPress={() => void control.sendCode()}
          />
        </Actions>
      </>
    );
  }

  return (
    <Card>
      <Heading level={2}>{phone ? w.sentToPhone : w.sentToEmail}</Heading>
      {sentTo.replaces ? <P>{phone ? w.sentToReplacesPhone : w.sentToReplacesEmail}</P> : null}
      {body}
      <P>{w.signInInstead}</P>
      <Actions>
        <Button label={wording.nav.signOut} variant="link" onPress={onSignOut} />
      </Actions>
    </Card>
  );
}
