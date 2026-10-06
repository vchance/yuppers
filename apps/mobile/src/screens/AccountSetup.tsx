import type { Account, ErrorCode } from '@yuppers/api-client';
import {
  codeWaitText,
  failureCode,
  identifierRefused,
  isComplete,
  languages,
  phoneOffered,
  pickLanguage,
  signInText,
  useResendReady,
  useSignInChannels,
  type Language,
} from '@yuppers/shared';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { TextInput } from 'react-native';

import {
  Actions,
  Button,
  Check,
  Choice,
  ErrorNote,
  Failure,
  Heading,
  Hint,
  Notice,
  P,
  Screen,
  TextField,
} from '../components/ui';
import { LegalLinks } from '../components/LegalLinks';
import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { DeleteAccount } from './DeleteAccount';

const languageOptions = languages.map((info) => ({ value: info.code, label: info.name }));

/**
 * Everything between "not signed in" and "ready to act": signing in with a
 * one-time code, then, for a new account, the profile. It shows whichever
 * step is next and nothing once both are done, so it can stand in for any
 * screen that needs an account.
 */
export function AccountSetup({ headingLevel = 1 }: { headingLevel?: 1 | 2 }) {
  const { wording } = useI18n();
  const { account } = useSession();

  if (!account) {
    return (
      <>
        <Heading level={headingLevel}>{wording.signIn.title}</Heading>
        <SignIn />
      </>
    );
  }
  if (!isComplete(account)) {
    return (
      <>
        <Heading level={headingLevel}>{wording.profile.firstTitle}</Heading>
        <P>{wording.profile.firstIntro}</P>
        <ProfileForm account={account} first />
      </>
    );
  }
  return null;
}

/**
 * Shows a screen only to a signed-in account that is ready to act; otherwise, the way to become one.
 * `signedOut` is shown under the way to sign in, to someone with no account yet.
 */
export function Gate({ children, signedOut }: { children: ReactNode; signedOut?: ReactNode }) {
  const { wording } = useI18n();
  const { ready, failure, account, retry } = useSession();

  if (!ready) {
    return (
      <Screen>
        <P>{wording.common.loading}</P>
      </Screen>
    );
  }
  if (failure) {
    return (
      <Screen>
        <Failure code={failure} />
        <Actions>
          <Button label={wording.common.tryAgain} onPress={retry} />
        </Actions>
      </Screen>
    );
  }
  if (!account || !isComplete(account)) {
    return (
      <Screen>
        <AccountSetup />
        {account ? null : signedOut}
        {/* An account exists from the first sign-in, before it has a name: it can be deleted from here. */}
        {account ? <DeleteAccount account={account} /> : null}
      </Screen>
    );
  }
  return children;
}

/**
 * Signing in: an email address or phone number, then the six-digit code sent
 * to it. The first time, this creates the account. The service answers a
 * request for a code the same way whether or not an account exists, and so
 * does this form. A phone number is asked for only where the service can
 * text it (`GET /v1/meta`); elsewhere one typed anyway is stopped here.
 *
 * While the code is on its way, the form says to look in the spam folder
 * too, for an email from the address the service names, and offers another
 * code only half a minute after the last (`useResendReady`).
 */
function SignIn() {
  const { wording, fmt, language, setLanguage } = useI18n();
  const { signedIn } = useSession();
  const w = wording.signIn;

  const [identifier, setIdentifier] = useState('');
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [resent, setResent] = useState(false);
  // When the latest code was sent, for offering another a while after.
  const [sentAt, setSentAt] = useState<number | null>(null);
  const resendReady = useResendReady(sentAt);
  // Said under the field, before or instead of what the service answered.
  const [problem, setProblem] = useState<string | null>(null);
  const channels = useSignInChannels(api);
  const text = signInText(w, channels, fmt);
  const phone = phoneOffered(channels);
  const codeInput = useRef<TextInput>(null);

  // Each step starts with the keyboard on the one thing it asks for.
  useEffect(() => {
    if (sentTo) codeInput.current?.focus();
  }, [sentTo]);

  async function requestCode(to: string, again: boolean) {
    setFailure(null);
    setProblem(null);
    setResent(false);
    if (identifierRefused(to, channels) === 'emailOnly') {
      setProblem(w.emailOnly);
      return;
    }
    setBusy(true);
    try {
      await api.requestCode(to);
      setSentTo(to);
      setSentAt(Date.now());
      setResent(again);
      // The button pressed goes away until another code may be asked for;
      // the keyboard goes back to the code.
      if (again) codeInput.current?.focus();
    } catch (error) {
      const code = failureCode(error);
      // The service's words for this one mention phone numbers.
      if (code === 'INVALID_IDENTIFIER' && !phone && channels) setProblem(w.invalidEmail);
      else setFailure(code);
    } finally {
      setBusy(false);
    }
  }

  async function signIn() {
    if (!sentTo) return;
    setBusy(true);
    setFailure(null);
    setResent(false);
    try {
      // The language on screen becomes a new account's language. The token
      // that comes back goes to the device's secure storage.
      await signedIn(await api.signIn(sentTo, code.trim(), language));
    } catch (error) {
      setFailure(failureCode(error));
      setBusy(false);
    }
  }

  if (!sentTo) {
    return (
      <>
        <P>{text.intro}</P>
        <TextField
          label={text.label}
          hint={text.hint}
          required
          inputMode="email"
          autoCapitalize="none"
          autoCorrect={false}
          autoComplete={phone ? 'username' : 'email'}
          textContentType={phone ? 'username' : 'emailAddress'}
          returnKeyType="send"
          value={identifier}
          onChangeText={(value) => {
            setIdentifier(value);
            setProblem(null);
          }}
          onSubmitEditing={() => void requestCode(identifier.trim(), false)}
        />
        {problem ? <ErrorNote>{problem}</ErrorNote> : <Failure code={failure} />}
        <Actions>
          <Button
            variant="primary"
            label={w.sendCode}
            disabled={busy}
            onPress={() => void requestCode(identifier.trim(), false)}
          />
        </Actions>
        {/* What a text message costs and how to stop them, said wherever a
            code can go to a phone number, before one is asked for. */}
        {phone ? (
          <>
            <Hint>{wording.privacy.sms}</Hint>
            <LegalLinks
              documents={['privacy']}
              testID="privacy-sms"
              section="text-messages"
              label={wording.privacy.smsLink}
            />
          </>
        ) : null}
        <LegalLinks />
        {/* Before anyone is signed in, the language is this device's to choose. */}
        <Choice<Language>
          label={wording.nav.language}
          value={language}
          options={languageOptions}
          onChange={setLanguage}
        />
      </>
    );
  }

  const wait = codeWaitText(w, sentTo, channels, fmt);
  return (
    <>
      <P>{fmt(w.codeSent, { identifier: sentTo })}</P>
      {wait ? <P>{wait}</P> : null}
      <TextField
        input={codeInput}
        label={w.codeLabel}
        hint={w.codeHint}
        required
        inputMode="numeric"
        autoComplete="one-time-code"
        textContentType="oneTimeCode"
        maxLength={6}
        returnKeyType="done"
        value={code}
        onChangeText={setCode}
        onSubmitEditing={() => void signIn()}
      />
      <Failure code={failure} />
      {resent && <Notice>{w.resent}</Notice>}
      {resendReady ? null : <Hint>{w.resendSoon}</Hint>}
      <Actions>
        <Button variant="primary" label={w.submit} disabled={busy} onPress={() => void signIn()} />
        {resendReady && (
          <Button label={w.resend} disabled={busy} onPress={() => void requestCode(sentTo, true)} />
        )}
      </Actions>
      <Actions>
        <Button
          variant="link"
          label={text.changeIdentifier}
          disabled={busy}
          onPress={() => {
            setSentTo(null);
            setSentAt(null);
            setCode('');
            setFailure(null);
            setProblem(null);
            setResent(false);
          }}
        />
      </Actions>
    </>
  );
}

/**
 * A name, a confirmation of being 18 or over, and a language. The first two
 * are needed before signing anything (DESIGN.md §14.1), so a new account is
 * asked for them straight after signing in; later the same form edits them.
 * The confirmation of age cannot be taken back, so once given it is stated,
 * not asked again.
 */
export function ProfileForm({ account, first }: { account: Account; first: boolean }) {
  const { wording, language: shown } = useI18n();
  const { setAccount } = useSession();
  const w = wording.profile;

  const [name, setName] = useState(account.display_name);
  const [language, setLanguage] = useState<Language>(
    first ? shown : pickLanguage([account.language]),
  );
  const [adult, setAdult] = useState(account.adult_confirmed);
  const [checked, setChecked] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [saved, setSaved] = useState(false);

  const nameMissing = name.trim() === '';
  const adultMissing = !account.adult_confirmed && !adult;

  async function submit() {
    setChecked(true);
    setSaved(false);
    if (nameMissing || adultMissing) return;
    setBusy(true);
    setFailure(null);
    try {
      const updated = await api.updateMe({
        display_name: name.trim(),
        language,
        adult_confirmed: adult ? true : null,
      });
      setAccount(updated);
      setSaved(true);
    } catch (error) {
      setFailure(failureCode(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <TextField
        label={w.nameLabel}
        hint={w.nameHint}
        required
        error={checked && nameMissing ? w.nameRequired : null}
        autoComplete="name"
        textContentType="name"
        maxLength={100}
        value={name}
        onChangeText={setName}
      />

      <Choice<Language>
        label={w.languageLabel}
        value={language}
        options={languageOptions}
        onChange={setLanguage}
      />

      {account.adult_confirmed ? (
        <P>{w.adultConfirmed}</P>
      ) : (
        <Check
          label={w.adultLabel}
          value={adult}
          onChange={setAdult}
          error={checked && adultMissing ? w.adultRequired : null}
        />
      )}

      <Failure code={failure} />
      <Actions>
        <Button
          variant="primary"
          label={first ? w.continue : w.save}
          disabled={busy}
          onPress={() => void submit()}
        />
      </Actions>
      {saved && !first && <Notice>{w.saved}</Notice>}
    </>
  );
}
