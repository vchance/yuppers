import type { Account } from '@yuppers/api-client';
import {
  PHONE_EXAMPLE,
  phoneAsTyped,
  shownIdentifier,
  useIdentifiers,
  type IdentifiersApi,
  type IdentifierSlot,
} from '@yuppers/shared';
import { useEffect } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { announce } from '../lib/accessibility';
import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { space, type, useColors } from '../lib/theme';
import { CombineOffer } from './CombineOffer';
import { ConsentCheckbox } from './ConsentCheckbox';
import { Actions, Button, Failure, Heading, Hint, Label, Notice, P, Panel, TextField } from './ui';

interface Props {
  account: Account;
  /** For tests: the service's calls. */
  client?: IdentifiersApi;
}

/**
 * The account's email address and phone number, each with Add or Change
 * and Remove, as on the web (`apps/web/src/components/Identifiers.tsx`).
 * Remove is offered only while the account has the other one, and says why
 * when it does not.
 */
export function Identifiers({ account, client = api }: Props) {
  const { wording, language } = useI18n();
  const { setAccount } = useSession();
  const colors = useColors();
  const w = wording.identifiers;
  const control = useIdentifiers(client, account, setAccount, wording, language);
  const { edit, offer } = control;

  useEffect(() => {
    if (control.done) announce(control.done);
  }, [control.done]);

  const rows: { slot: IdentifierSlot; label: string; value: string | null | undefined }[] = [
    { slot: 'email', label: wording.profile.emailLabel, value: account.email },
    { slot: 'phone', label: wording.profile.phoneLabel, value: account.phone },
  ];

  return (
    <View style={styles.section}>
      <Heading level={2}>{w.heading}</Heading>
      <Hint>{w.intro}</Hint>
      {control.done ? <Notice>{control.done}</Notice> : null}
      {rows.map(({ slot, label, value }) => {
        const removable = control.removable(slot);
        return (
          <View key={slot} style={styles.detail}>
            <Label>{label}</Label>
            {/* A phone number reads left to right in every language. */}
            <Text selectable style={[type.body, styles.ltr, { color: colors.text }]}>
              {value ? shownIdentifier(value) : w.none}
            </Text>
            <Actions>
              <Button
                testID={`${slot}-${value ? 'change' : 'add'}`}
                label={value ? w.change : w.add}
                accessibilityLabel={`${value ? w.change : w.add}: ${label}`}
                disabled={control.busy}
                onPress={() => control.start(slot, value ? 'change' : 'add')}
              />
              {value ? (
                <Button
                  testID={`${slot}-remove`}
                  label={w.remove}
                  accessibilityLabel={`${w.remove}: ${label}`}
                  hint={removable ? undefined : slot === 'email' ? w.onlyEmail : w.onlyPhone}
                  disabled={control.busy || !removable}
                  onPress={() => control.start(slot, 'remove')}
                />
              ) : null}
            </Actions>
            {value && !removable ? <Hint>{slot === 'email' ? w.onlyEmail : w.onlyPhone}</Hint> : null}
          </View>
        );
      })}
      {edit ? <EditPanel control={control} account={account} /> : null}
      {offer ? (
        <CombineOffer
          offer={offer}
          busy={control.busy}
          failure={control.failure}
          onCombine={() => void control.combine()}
          onCancel={control.dismissOffer}
        />
      ) : null}
    </View>
  );
}

function EditPanel({
  control,
  account,
}: {
  control: ReturnType<typeof useIdentifiers>;
  account: Account;
}) {
  const { wording, fmt } = useI18n();
  const w = wording.identifiers;
  const edit = control.edit;
  if (!edit) return null;
  const consent = control.codeConsent;

  const title =
    edit.action === 'remove'
      ? edit.slot === 'email'
        ? w.removeEmailTitle
        : w.removePhoneTitle
      : edit.action === 'change'
        ? edit.slot === 'email'
          ? w.changeEmailTitle
          : w.changePhoneTitle
        : edit.slot === 'email'
          ? w.addEmailTitle
          : w.addPhoneTitle;

  const consentBox = consent.shown ? (
    <>
      <ConsentCheckbox
        wording={wording.smsCode.verifyNumber}
        checked={consent.checked}
        onChange={consent.setChecked}
        disabled={control.busy}
      />
      {consent.missing ? <Hint>{wording.smsCode.tickToSend}</Hint> : null}
    </>
  ) : null;

  const codeStep = (sentTo: string, label: string) => (
    <>
      <P>{fmt(w.codeSent, { identifier: shownIdentifier(sentTo) })}</P>
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
        <Button label={wording.common.cancel} disabled={control.busy} onPress={control.cancel} />
      </Actions>
    </>
  );

  if (edit.action === 'remove') {
    const staying = shownIdentifier(edit.staying);
    return (
      <Panel title={title}>
        {edit.step === 'explain' ? (
          <>
            <P>{fmt(w.removeIntro, { staying })}</P>
            <P>{edit.slot === 'phone' ? w.removePhoneNote : w.removeEmailNote}</P>
            {consentBox}
            <Failure code={control.failure} />
            <Actions>
              <Button
                label={fmt(w.sendRemovalCode, { staying })}
                variant="primary"
                disabled={control.busy || consent.missing}
                hint={consent.missing ? wording.smsCode.tickToSend : undefined}
                onPress={() => void control.sendCode()}
              />
              <Button
                label={wording.common.cancel}
                disabled={control.busy}
                onPress={control.cancel}
              />
            </Actions>
          </>
        ) : (
          codeStep(edit.staying, w.removeConfirm)
        )}
      </Panel>
    );
  }

  if (edit.step === 'prove' || edit.step === 'proveCode') {
    const to = shownIdentifier(edit.to);
    const alternative = control.proveAlternative;
    return (
      <Panel title={title}>
        {edit.step === 'prove' ? (
          <>
            <P>{fmt(w.proveIntro, { identifier: to })}</P>
            {consentBox}
            <Failure code={control.failure} />
            <Actions>
              <Button
                label={fmt(w.proveSend, { identifier: to })}
                variant="primary"
                disabled={control.busy || consent.missing}
                hint={consent.missing ? wording.smsCode.tickToSend : undefined}
                onPress={() => void control.sendCode()}
              />
              {alternative ? (
                <Button
                  label={fmt(w.proveOther, { identifier: shownIdentifier(alternative) })}
                  disabled={control.busy}
                  onPress={control.proveElsewhere}
                />
              ) : null}
              <Button
                label={wording.common.cancel}
                disabled={control.busy}
                onPress={control.cancel}
              />
            </Actions>
          </>
        ) : (
          codeStep(edit.to, w.proveConfirm)
        )}
      </Panel>
    );
  }

  const current = edit.slot === 'email' ? account.email : account.phone;
  return (
    <Panel title={title}>
      {edit.step === 'enter' ? (
        <>
          {current ? <P>{fmt(w.changeNote, { identifier: shownIdentifier(current) })}</P> : null}
          {current && edit.slot === 'email' ? (
            <P>{fmt(w.changeEmailTold, { identifier: current })}</P>
          ) : null}
          {edit.slot === 'email' ? (
            <TextField
              label={w.newEmailLabel}
              required
              keyboardType="email-address"
              autoComplete="email"
              textContentType="emailAddress"
              autoCapitalize="none"
              value={control.input}
              onChangeText={control.setInput}
            />
          ) : (
            <TextField
              label={w.newPhoneLabel}
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
          )}
          <Failure code={control.failure} />
          {consentBox}
          <Actions>
            <Button
              label={w.sendCode}
              variant="primary"
              disabled={control.busy || consent.missing}
              hint={consent.missing ? wording.smsCode.tickToSend : undefined}
              onPress={() => void control.sendCode()}
            />
            <Button label={wording.common.cancel} disabled={control.busy} onPress={control.cancel} />
          </Actions>
        </>
      ) : (
        codeStep(edit.identifier, w.confirm)
      )}
    </Panel>
  );
}

const styles = StyleSheet.create({
  section: { gap: space.s },
  detail: { gap: space.xs },
  ltr: { writingDirection: 'ltr' },
});
