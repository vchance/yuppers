import type { Account } from '@yuppers/api-client';
import { useAccountDeletion, useSignInChannels, type CodeChannel } from '@yuppers/shared';
import { useRouter } from 'expo-router';
import { useEffect, useRef, useState } from 'react';
import type { TextInput } from 'react-native';

import {
  Actions,
  Button,
  Choice,
  Failure,
  Heading,
  Hint,
  Lines,
  Notice,
  P,
  Panel,
  TextField,
} from '../components/ui';
import { ConsentCheckbox } from '../components/ConsentCheckbox';
import { HelpLink } from '../components/HelpLink';
import { LegalLinks } from '../components/LegalLinks';
import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';

/**
 * Deleting the account, from the account screen (DESIGN.md §4.1). It cannot
 * be undone, so it is said in full first, what goes and what stays, then
 * proved with a one-time code, then asked once more.
 */
export function DeleteAccount({ account }: { account: Account }) {
  const { wording } = useI18n();
  const w = wording.deletion;
  const [open, setOpen] = useState(false);

  return (
    <>
      <Heading level={2}>{w.heading}</Heading>
      <HelpLink place="deletion" />
      <LegalLinks
        documents={['privacy']}
        testID="privacy-deletion"
        section="deleting-your-account"
        label={wording.privacy.deletion}
      />
      {open ? (
        <Steps account={account} onCancel={() => setOpen(false)} />
      ) : (
        <Actions>
          <Button label={w.open} onPress={() => setOpen(true)} />
        </Actions>
      )}
    </>
  );
}

function Steps({ account, onCancel }: { account: Account; onCancel(): void }) {
  const { wording, fmt, language } = useI18n();
  const { forget } = useSession();
  const router = useRouter();
  const w = wording.deletion;

  // A phone number is offered for the code only where the service can text it.
  const channels = useSignInChannels(api);
  const deletion = useAccountDeletion(
    api,
    account,
    async () => {
      // The service has ended every session; the token this device held is
      // taken out of its secure storage first, and only then is it back to
      // the first screen, where what is left is the way to sign in. In the
      // other order, a first screen that was not already beneath this one (the
      // account screen opened by a direct link) would appear while the account
      // was still known and take the notice that it was deleted as read
      // (`AccountDeleted`).
      await forget();
      router.dismissTo('/');
    },
    channels,
    language,
  );
  const { step, preview, destination, busy } = deletion;
  const identifier = destination?.identifier ?? '';
  const consent = deletion.codeConsent;

  // The code step starts with the keyboard on the one thing it asks for.
  const codeInput = useRef<TextInput>(null);
  useEffect(() => {
    if (step === 'code') codeInput.current?.focus();
  }, [step]);

  if (step === 'confirm') {
    return (
      <Panel key="confirm" title={w.confirmHeading}>
        <P>{w.confirmBody}</P>
        <Failure code={deletion.failure} />
        <Actions>
          <Button
            variant="primary"
            label={w.confirm}
            disabled={busy}
            onPress={() => void deletion.confirm()}
          />
          <Button label={w.back} disabled={busy} onPress={deletion.back} />
          <Button
            variant="link"
            label={wording.common.cancel}
            disabled={busy}
            onPress={onCancel}
          />
        </Actions>
      </Panel>
    );
  }

  if (step === 'code') {
    return (
      <Panel key="code" title={w.heading}>
        <P>{fmt(w.codeSent, { identifier })}</P>
        <TextField
          input={codeInput}
          label={wording.signIn.codeLabel}
          hint={wording.signIn.codeHint}
          required
          error={deletion.codeMissing ? w.codeRequired : null}
          inputMode="numeric"
          autoComplete="one-time-code"
          textContentType="oneTimeCode"
          maxLength={6}
          returnKeyType="done"
          value={deletion.code}
          onChangeText={deletion.setCode}
          onSubmitEditing={deletion.review}
        />
        <Failure code={deletion.failure} />
        {deletion.resent && <Notice>{wording.signIn.resent}</Notice>}
        <Actions>
          <Button variant="primary" label={w.continue} disabled={busy} onPress={deletion.review} />
          <Button
            label={wording.signIn.resend}
            disabled={busy}
            onPress={() => void deletion.sendCode()}
          />
          <Button
            variant="link"
            label={wording.common.cancel}
            disabled={busy}
            onPress={onCancel}
          />
        </Actions>
      </Panel>
    );
  }

  const open = preview ? preview.open_proposals + preview.agreements_in_force : 0;
  return (
    <Panel key="explain" title={w.intro}>
      <Heading level={4}>{w.deletedHeading}</Heading>
      <Lines>
        <P>{w.deletedAccount}</P>
        <P>{w.deletedData}</P>
        <P>{w.signUpAgain}</P>
      </Lines>

      <Heading level={4}>{w.exchangesHeading}</Heading>
      {!preview && !deletion.previewFailure ? <P>{wording.common.loading}</P> : null}
      {!preview && deletion.previewFailure ? (
        <>
          <Failure code={deletion.previewFailure} />
          <P>{w.loadFailed}</P>
          <Actions>
            <Button label={wording.common.tryAgain} onPress={deletion.loadPreview} />
          </Actions>
        </>
      ) : null}
      {preview ? (
        <Lines>
          {preview.drafts + open === 0 ? <P>{w.nothingOpen}</P> : null}
          {preview.drafts > 0 ? <P>{fmt(w.drafts, { count: preview.drafts })}</P> : null}
          {preview.open_proposals > 0 ? (
            <P>{fmt(w.openProposals, { count: preview.open_proposals })}</P>
          ) : null}
          {preview.agreements_in_force > 0 ? (
            <P>{fmt(w.agreementsInForce, { count: preview.agreements_in_force })}</P>
          ) : null}
          {preview.agreements_in_force > 0 ? <P>{w.agreementsStand}</P> : null}
          {open > 0 ? <P>{w.otherPartyTold}</P> : null}
        </Lines>
      ) : null}

      <Heading level={4}>{w.keptHeading}</Heading>
      <Lines>
        <P>{w.keptAgreements}</P>
        <P>{w.keptReports}</P>
      </Lines>

      {deletion.destinations.length > 1 ? (
        <>
          <P>{w.codeChoice}</P>
          <Choice<CodeChannel>
            label={w.sendTo}
            value={destination?.channel ?? null}
            options={deletion.destinations.map((option) => ({
              value: option.channel,
              label: option.identifier,
            }))}
            onChange={deletion.choose}
            disabled={busy}
          />
        </>
      ) : (
        <P>{fmt(w.codeIntro, { identifier })}</P>
      )}

      {/* A code by text only once the box beside the number is ticked. */}
      {consent.shown ? (
        <ConsentCheckbox
          wording={wording.smsCode.deleteAccount}
          checked={consent.checked}
          onChange={consent.setChecked}
          disabled={busy}
        />
      ) : null}
      {consent.missing ? <Hint>{wording.smsCode.tickToSend}</Hint> : null}
      <Failure code={deletion.failure} />
      <Actions>
        <Button
          variant="primary"
          label={w.sendCode}
          disabled={busy || !preview || !destination || consent.missing}
          hint={consent.missing ? wording.smsCode.tickToSend : undefined}
          onPress={() => void deletion.sendCode()}
        />
        <Button label={wording.common.cancel} disabled={busy} onPress={onCancel} />
      </Actions>
    </Panel>
  );
}
