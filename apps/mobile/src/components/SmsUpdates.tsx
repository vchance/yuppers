import type { ExchangeView as Exchange } from '@yuppers/api-client';
import {
  consentPieces,
  defaultLanguage,
  maskPhone,
  staticPagePath,
  useSmsUpdates,
  type SmsUpdatesApi,
} from '@yuppers/shared';
import { useEffect, useState } from 'react';
import { Linking, Pressable, StyleSheet, Text, View } from 'react-native';

import { announce } from '../lib/accessibility';
import { WEB_URL } from '../lib/config';
import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { space, TOUCH_TARGET, type, useColors } from '../lib/theme';
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
 * US number, checked with a code by text, then a box beside the consent
 * wording, word for word as the terms quote it, and Save. Shown only where
 * the service texts updates, on an agreement sent and not closed
 * (`useSmsUpdates` decides).
 */
export function SmsUpdates({ exchange, client = api }: Props) {
  const { wording, fmt, language } = useI18n();
  const session = useSession();
  const colors = useColors();
  const w = wording.smsUpdates;
  // A number added here is the account's from now on, on every screen.
  const control = useSmsUpdates(client, exchange, language, session.setAccount);
  const [phone, setPhone] = useState('');
  const [code, setCode] = useState('');
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
            autoComplete="tel"
            textContentType="telephoneNumber"
            value={phone}
            onChangeText={setPhone}
          />
          <Failure code={control.failure} />
          <Hint>{wording.privacy.sms}</Hint>
          <Actions>
            <Button
              label={w.sendCode}
              variant="primary"
              disabled={control.busy}
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
          <Failure code={control.failure} />
          <Actions>
            <Button
              label={w.addPhone}
              variant="primary"
              disabled={control.busy}
              onPress={() => void control.verify(code)}
            />
            <Button
              label={w.changePhone}
              variant="link"
              disabled={control.busy}
              onPress={() => {
                setCode('');
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
          <View style={styles.consent}>
            <Pressable
              accessibilityRole="checkbox"
              accessibilityLabel={w.consent}
              accessibilityLanguage={language}
              accessibilityState={{ checked: control.checked, disabled: control.busy }}
              aria-checked={control.checked}
              disabled={control.busy}
              onPress={() => control.setChecked(!control.checked)}
              style={styles.box}>
              <View
                style={[
                  styles.square,
                  {
                    borderColor: control.checked ? colors.primary : colors.border,
                    backgroundColor: control.checked ? colors.primary : colors.background,
                  },
                ]}>
                {control.checked ? (
                  <Text
                    aria-hidden
                    importantForAccessibility="no"
                    style={[type.body, { color: colors.onPrimary }]}>
                    ✓
                  </Text>
                ) : null}
              </View>
            </Pressable>
            <Text
              accessibilityLanguage={language}
              style={[type.body, styles.consentText, { color: colors.text }]}>
              {consentPieces(w.consent).map((piece, index) =>
                'url' in piece ? (
                  <Text
                    key={index}
                    accessibilityRole="link"
                    accessibilityHint={wording.help.inBrowser}
                    onPress={() => openInBrowser(piece.url)}
                    style={[styles.link, { color: colors.primary }]}>
                    {piece.url}
                  </Text>
                ) : (
                  piece.text
                ),
              )}
            </Text>
          </View>
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

const styles = StyleSheet.create({
  consent: { flexDirection: 'row', alignItems: 'flex-start', gap: space.m },
  box: {
    minHeight: TOUCH_TARGET,
    minWidth: TOUCH_TARGET,
    alignItems: 'center',
    justifyContent: 'center',
  },
  square: {
    width: 28,
    height: 28,
    borderWidth: 2,
    borderRadius: 4,
    alignItems: 'center',
    justifyContent: 'center',
  },
  consentText: { flex: 1, paddingTop: space.s },
  link: { textDecorationLine: 'underline' },
});
