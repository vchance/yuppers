import type { Account, ErrorCode } from '@yuppers/api-client';
import {
  codeWaitText,
  failureCode,
  formatPhone,
  identifierRefused,
  identifierToSend,
  isComplete,
  languages,
  phoneAsTyped,
  phoneOffered,
  phoneRefused,
  pickLanguage,
  readsAsPhone,
  signInText,
  smsCodeConsent,
  useResendReady,
  useSignInChannels,
  useSmsCodeConsentBox,
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
import { BrandHero } from '../components/Brand';
import { ConsentCheckbox } from '../components/ConsentCheckbox';
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
 *
 * As a screen of its own it opens with the brand (`BrandHero`), taking over
 * from the launch screen: the first thing someone sees of the product should
 * look like the product. Below an invitation, which has a heading of its own,
 * the steps open plain.
 */
export function AccountSetup({ headingLevel = 1 }: { headingLevel?: 1 | 2 }) {
  const { wording } = useI18n();
  const { account } = useSession();
  const standalone = headingLevel === 1;

  if (!account) {
    return (
      <>
        {standalone ? <BrandHero /> : null}
        <Heading level={headingLevel}>{wording.signIn.title}</Heading>
        <SignIn />
      </>
    );
  }
  if (!isComplete(account)) {
    return (
      <>
        {standalone ? <BrandHero /> : null}
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
 * As soon as what is typed reads as a phone number, an unticked box appears
 * beside the words the terms quote (`smsCode.signIn`), and "Send code"
 * waits for it, saying why: a code goes by text only once it is ticked
 * (`sms-code-consent.ts`). Never ticked to begin with, and unticked again
 * whenever the number changes.
 *
 * A US number is typed the way it is written there, `(856) 548-8780`,
 * with or without +1, and is sent in E.164 (`phone.ts`); leaving the field
 * writes it that way. A number of a country the service does not text is
 * stopped here, in the service's own words.
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
  // The box, while what is typed would be texted a code.
  const typed = identifier.trim();
  const texted = readsAsPhone(typed) && identifierRefused(typed, channels) === null;
  // The box is for the number, not for how it is written: writing it the
  // American way on leaving the field leaves the box as it was.
  const consent = useSmsCodeConsentBox(texted ? identifierToSend(typed) : null);

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
    const refused = phoneRefused(to, channels);
    if (refused) {
      setFailure(refused);
      return;
    }
    const target = identifierToSend(to);
    setBusy(true);
    try {
      await api.requestCode(target, consent.checked ? smsCodeConsent(language) : undefined);
      setSentTo(target);
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
          onBlur={() => setIdentifier(phoneAsTyped)}
          onSubmitEditing={() => {
            if (!consent.missing) void requestCode(identifier.trim(), false);
          }}
        />
        {problem ? <ErrorNote>{problem}</ErrorNote> : <Failure code={failure} />}
        {consent.shown ? (
          <ConsentCheckbox
            wording={wording.smsCode.signIn}
            checked={consent.checked}
            onChange={consent.setChecked}
            disabled={busy}
          />
        ) : null}
        {consent.missing ? <Hint>{wording.smsCode.tickToSend}</Hint> : null}
        <Actions>
          <Button
            variant="primary"
            label={w.sendCode}
            disabled={busy || consent.missing}
            hint={consent.missing ? wording.smsCode.tickToSend : undefined}
            onPress={() => void requestCode(identifier.trim(), false)}
          />
        </Actions>
        {/* Where a code can go to a phone number, the policy on texts. */}
        {phone ? (
          <LegalLinks
            documents={['privacy']}
            testID="privacy-sms"
            section="text-messages"
            label={wording.privacy.smsLink}
          />
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
      <P>{fmt(w.codeSent, { identifier: formatPhone(sentTo) })}</P>
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
            // Back to the number, the box is unticked: it is never remembered.
            consent.setChecked(false);
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
  const [detail, setDetail] = useState(account.notification_detail);
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
        notification_detail: detail,
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

      {!first && <Check label={w.detailLabel} value={detail} onChange={setDetail} />}
      {!first && <P>{w.detailHint}</P>}

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
