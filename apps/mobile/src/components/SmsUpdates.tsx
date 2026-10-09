import type { ExchangeView as Exchange } from '@yuppers/api-client';
import {
  defaultLanguage,
  maskPhone,
  PHONE_EXAMPLE,
  phoneAsTyped,
  staticPagePath,
  useSmsUpdates,
  type SmsUpdatesApi,
} from '@yuppers/shared';
import { useEffect, useState } from 'react';
import { Linking } from 'react-native';

import { announce } from '../lib/accessibility';
import { WEB_URL } from '../lib/config';
import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { ConsentCheckbox } from './ConsentCheckbox';
import { Actions, Button, Card, Failure, Heading, Hint, Notice, P, TextField } from './ui';

/** Opens an address in the system's browser; a device without one has nothing better. */
function openInBrowser(url: string): void {
  Linking.openURL(url).catch(() => {});
}

interface Props {
  exchange: Exchange;
  /** For tests: the service's calls. */
  client?: SmsUpdatesApi;
}

/**
 * "Text updates" on an agreement (DESIGN.md §12, "Yuppers.app agreement
 * updates"), as on the web (`apps/web/src/components/SmsUpdates.tsx`): add a
 * US number, checked with a code by text once the box beside it is ticked
 * (`smsCode.verifyNumber`), then a box beside the consent wording, word for
 * word as the terms quote it, and Save. Shown only where
 * the service texts updates, on an agreement sent and not closed
 * (`useSmsUpdates` decides).
 */
export function SmsUpdates({ exchange, client = api }: Props) {
  const { wording, fmt, language } = useI18n();
  const session = useSession();
  const w = wording.smsUpdates;
  // A number added here is the account's from now on, on every screen.
  const control = useSmsUpdates(
    client,
    exchange,
    language,
    session.setAccount,
    session.account?.email ?? null,
  );
  const [phone, setPhone] = useState('');
  const [code, setCode] = useState('');
  const [proofCode, setProofCode] = useState('');
  const { step, standing, pending, saved, phoneAdded } = control;

  const number = standing?.phone ? maskPhone(standing.phone) : '';
  const said = saved === 'on' ? fmt(w.on, { phone: number }) : saved === 'off' ? w.off : null;
  useEffect(() => {
    if (said) announce(said);
  }, [said]);

  if (step === 'hidden' || step === 'loading') return null;

  const optIn = `${WEB_URL.replace(/\/+$/, '')}${staticPagePath('sms-opt-in', language, defaultLanguage)}`;

  return (
    <Card>
      <Heading level={2}>{w.heading}</Heading>
      <P>{w.intro}</P>

      {step === 'addPhone' && (
        <>
          <P>{w.addPhoneIntro}</P>
          <TextField
            label={w.phoneLabel}
            hint={w.phoneHint}
            required
            error={control.invalidPhone ? w.phoneInvalid : null}
            keyboardType="phone-pad"
            autoComplete="tel-national"
            textContentType="telephoneNumber"
            placeholder={PHONE_EXAMPLE}
            value={phone}
            onChangeText={(value) => {
              setPhone(value);
              // What was ticked was for the number as it was.
              control.setCodeConsent(false);
            }}
            // Written the American way on leaving: the same number, so the
            // box stays as it was.
            onBlur={() => setPhone(phoneAsTyped)}
          />
          <Failure code={control.failure} />
          <ConsentCheckbox
            wording={wording.smsCode.verifyNumber}
            checked={control.codeConsent}
            onChange={control.setCodeConsent}
            disabled={control.busy}
          />
          {control.codeConsent ? null : <Hint>{wording.smsCode.tickToSend}</Hint>}
          <Actions>
            <Button
              label={w.sendCode}
              variant="primary"
              disabled={control.busy || !control.codeConsent}
              hint={control.codeConsent ? undefined : wording.smsCode.tickToSend}
              onPress={() => void control.requestCode(phone)}
            />
          </Actions>
        </>
      )}

      {step === 'enterCode' && pending && (
        <>
          <P>{fmt(w.codeSent, { phone: maskPhone(pending) })}</P>
          <TextField
            label={w.codeLabel}
            hint={wording.signIn.codeHint}
            required
            keyboardType="number-pad"
            autoComplete="one-time-code"
            textContentType="oneTimeCode"
            maxLength={6}
            value={code}
            onChangeText={setCode}
          />
          {control.proofTo ? (
            <>
              <P>{fmt(w.proofCodeSent, { email: control.proofTo })}</P>
              <TextField
                label={w.proofCodeLabel}
                hint={wording.signIn.codeHint}
                required
                keyboardType="number-pad"
                maxLength={6}
                value={proofCode}
                onChangeText={setProofCode}
              />
            </>
          ) : null}
          <Failure code={control.failure} />
          <Actions>
            <Button
              label={w.addPhone}
              variant="primary"
              disabled={control.busy}
              onPress={() => void control.verify(code, proofCode)}
            />
            <Button
              label={w.changePhone}
              variant="link"
              disabled={control.busy}
              onPress={() => {
                setCode('');
                setProofCode('');
                control.changePhone();
              }}
            />
          </Actions>
        </>
      )}

      {step === 'consent' && (
        <>
          {phoneAdded && (
            <Notice>{fmt(w.phoneAdded, { phone: maskPhone(phoneAdded) })}</Notice>
          )}
          <ConsentCheckbox
            wording={w.consent}
            checked={control.checked}
            onChange={control.setChecked}
            disabled={control.busy}
          />
          <Failure code={control.failure} />
          <Actions>
            <Button
              label={w.save}
              variant="primary"
              disabled={control.busy}
              onPress={() => void control.save()}
            />
          </Actions>
          {said ? (
            <Notice quiet>{said}</Notice>
          ) : standing?.on ? (
            <Hint>{fmt(w.on, { phone: number })}</Hint>
          ) : null}
        </>
      )}

      {step === 'optedOut' && <Notice quiet>{fmt(w.optedOut, { phone: number })}</Notice>}

      <Actions>
        <Button
          label={w.howItWorks}
          variant="link"
          hint={wording.help.inBrowser}
          onPress={() => openInBrowser(optIn)}
        />
      </Actions>
    </Card>
  );
}
