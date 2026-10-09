import type { ErrorCode } from '@yuppers/api-client';
import {
  failureCode,
  invitationTokenIn,
  isComplete,
  type InvitationPreview,
} from '@yuppers/shared';
import * as Clipboard from 'expo-clipboard';
import { useRouter } from 'expo-router';
import { useEffect, useRef, useState } from 'react';

import { YupCard } from '../components/Callouts';
import { InvitationAddress } from '../components/InvitationAddress';
import { TermsView } from '../components/TermsView';
import {
  Actions,
  Button,
  Card,
  ErrorNote,
  Failure,
  Heading,
  Hint,
  Notice,
  P,
  Screen,
  TextField,
  Written,
} from '../components/ui';
import { useI18n, useSession } from '../lib/context';
import { forgetInvitation, holdInvitation, useHeldInvitation } from '../lib/invitation';
import { api } from '../lib/session';
import { AccountSetup } from './AccountSetup';
import { InvitationReport } from './InvitationReport';

/**
 * Where an invitation is opened. The link's token reaches this screen in
 * memory, never in its address: handed over by the system when the link
 * opened the app, or pasted here by the person.
 */
export function InvitationScreen() {
  const { account } = useSession();
  const held = useHeldInvitation();
  // The token this screen is showing. Forgetting the held one, once it is
  // spent, does not take the screen away from under the person.
  const [token, setToken] = useState(held);
  if (held && held !== token) setToken(held);

  if (!token) return <PasteInvitation onToken={setToken} />;
  // Whoever signs in, or out, starts from the top: nothing one person was
  // shown stays on the screen for the next.
  return (
    <Invitation
      key={`${token} ${account?.id ?? ''}`}
      token={token}
      onAnother={() => setToken(null)}
    />
  );
}

/** For a link that did not open the app by itself: paste it. */
function PasteInvitation({ onToken }: { onToken(token: string): void }) {
  const { wording } = useI18n();
  const w = wording.mobile.openInvitation;
  const [text, setText] = useState('');
  const [invalid, setInvalid] = useState(false);

  function open(given: string) {
    const token = invitationTokenIn(given);
    if (!token) {
      setInvalid(true);
      return;
    }
    holdInvitation(token);
    onToken(token);
  }

  async function paste() {
    let pasted = '';
    try {
      pasted = await Clipboard.getStringAsync();
    } catch {
      // Nothing to paste, or the person said no.
    }
    if (pasted === '') return;
    setInvalid(false);
    // A pasted invitation opens straight away; anything else is shown so it can be corrected.
    if (invitationTokenIn(pasted)) open(pasted);
    else setText(pasted);
  }

  return (
    <Screen>
      <Heading>{w.title}</Heading>
      <P>{w.intro}</P>
      <TextField
        label={w.label}
        hint={w.hint}
        error={invalid ? w.invalid : null}
        autoCapitalize="none"
        autoCorrect={false}
        autoComplete="off"
        spellCheck={false}
        inputMode="url"
        multiline
        value={text}
        onChangeText={(next) => {
          setText(next);
          setInvalid(false);
        }}
      />
      <Actions>
        <Button variant="primary" label={w.open} onPress={() => open(text)} />
        <Button label={w.paste} onPress={() => void paste()} />
      </Actions>
    </Screen>
  );
}

/**
 * An invitation, as the account signed in sees it. Someone signed out is
 * shown only that a yup is waiting and the way to sign in, the same for every
 * link, live or dead: the service answers nothing about a link to someone
 * signed out, so a blocked person has nothing to compare with the dead link
 * they see signed in (DESIGN.md §9). Responding means claiming the
 * invitation, which takes the invited party's place in the exchange
 * (DESIGN.md §8). Reading claims nothing, and holding the link proves
 * nothing about who is holding it.
 */
function Invitation({ token, onAnother }: { token: string; onAnother(): void }) {
  const { wording } = useI18n();
  const { account, ready } = useSession();
  const w = wording.invitation;

  if (!ready) {
    return (
      <Screen>
        <P>{wording.common.loading}</P>
      </Screen>
    );
  }
  if (!account) {
    // The same screen for every link: nothing here depends on the token.
    return (
      <Screen>
        <Heading>{w.signedOutTitle}</Heading>
        <P>{w.signInToRead}</P>
        <AccountSetup headingLevel={2} />
      </Screen>
    );
  }
  return <Proposal token={token} onAnother={onAnother} />;
}

/** The proposal behind an invitation, for the account signed in. */
function Proposal({ token, onAnother }: { token: string; onAnother(): void }) {
  const { wording, fmt, moment } = useI18n();
  const { account, signOut } = useSession();
  const router = useRouter();
  const w = wording.invitation;

  const [preview, setPreview] = useState<InvitationPreview | null>(null);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [spent, setSpent] = useState(false);
  const [responding, setResponding] = useState(false);
  const claiming = useRef(false);

  useEffect(() => {
    let cancelled = false;
    api.previewInvitation(token).then(
      (found) => {
        if (!cancelled) setPreview(found);
      },
      (error: unknown) => {
        if (cancelled) return;
        // A session that has ended signs the screen out by itself.
        const code = failureCode(error);
        if (code === 'INVITATION_UNAVAILABLE') setSpent(true);
        else setFailure(code);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [token]);

  const able = account !== null && isComplete(account);

  // A link that no longer shows its proposal has usually been used, and
  // people come back to the message it arrived in. The service is asked
  // whether the place it gave is already this account's, which takes
  // nothing whatever the answer: the person who used the link is taken to
  // the exchange, and anyone else gets the same refusal as before. Only the
  // button below ever claims.
  useEffect(() => {
    if (!spent) return;
    if (!able) {
      forgetInvitation();
      return;
    }
    if (claiming.current) return;
    claiming.current = true;
    api.invitationAlreadyYours(token).then(
      (exchange) => {
        forgetInvitation();
        router.replace(`/exchanges/${exchange.id}`);
      },
      (error: unknown) => {
        forgetInvitation();
        claiming.current = false;
        setFailure(failureCode(error));
      },
    );
  }, [spent, able, token, router]);

  // Once the person has asked to respond and has an account that can, claim.
  useEffect(() => {
    if (!responding || !able || claiming.current) return;
    claiming.current = true;
    api.claimInvitation(token).then(
      (exchange) => {
        forgetInvitation();
        router.replace(`/exchanges/${exchange.id}`);
      },
      (error: unknown) => {
        const code = failureCode(error);
        if (code === 'INVITATION_UNAVAILABLE') forgetInvitation();
        claiming.current = false;
        setResponding(false);
        setFailure(code);
      },
    );
  }, [responding, able, token, router]);

  const sender = preview?.revision.terms.party_a_name ?? '';

  // A link that cannot be read is refused at the top of an otherwise empty
  // screen. A claim is refused next to the button that asked for it, which
  // is below the whole proposal.
  const refused: ErrorCode | null =
    failure ?? (spent && !able ? 'INVITATION_UNAVAILABLE' : null);
  const refusal = refused && (
    <>
      {/* The only refusal a claim gives for this reason is opening one's own link. */}
      {refused === 'ACTION_NOT_ALLOWED' ? (
        <ErrorNote>{w.ownInvitation}</ErrorNote>
      ) : (
        <Failure code={refused} />
      )}
      <Actions>
        {refused === 'INVITATION_NOT_FOR_YOU' && (
          <Button label={wording.nav.signOut} onPress={() => void signOut()} />
        )}
        <Button label={wording.common.goHome} onPress={() => router.dismissTo('/')} />
        {!preview && (
          <Button
            label={wording.mobile.openInvitation.title}
            onPress={() => {
              forgetInvitation();
              onAnother();
            }}
          />
        )}
      </Actions>
    </>
  );

  return (
    <Screen>
      <Heading>{w.title}</Heading>

      {!preview && refusal}
      {!preview && !refused && <P>{wording.common.loading}</P>}

      {preview && (
        <>
          <YupCard
            from={sender}
            to={preview.revision.terms.party_b_name || wording.party.other}
          />
          {/* Whoever holds a link that names nobody can open it, so its sender
              confirms them before they can do more than sign (DESIGN.md §8). */}
          <P>
            {fmt(preview.bound ? w.introSignedIn : wording.claimant.invitationIntroSignedIn, {
              name: sender,
            })}
          </P>
          <P>{w.notBinding}</P>

          <Card>
            <Hint>{fmt(wording.home.reference, { code: preview.display_code })}</Hint>
            {preview.revision.note ? (
              <>
                <Heading level={2}>{fmt(w.noteHeading, { name: sender })}</Heading>
                <Written>{preview.revision.note}</Written>
              </>
            ) : null}
            <TermsView
              terms={preview.revision.terms}
              currency={preview.currency}
              timezone={preview.timezone}
              you={null}
            />
            <Hint>{fmt(w.expires, { date: moment(preview.revision.expires_at) })}</Hint>
          </Card>

          {preview.bound && !preview.sent_to && <P>{w.boundSignedIn}</P>}
          {refusal}

          {/* Sent to an address this account does not have: add it first. */}
          {preview.sent_to && (
            <InvitationAddress
              token={token}
              sentTo={preview.sent_to}
              onOpened={(exchange) => {
                forgetInvitation();
                router.replace(`/exchanges/${exchange.id}`);
              }}
              onSignOut={() => void signOut()}
            />
          )}

          {responding && able && <Notice>{w.opening}</Notice>}
          {responding && !able && <AccountSetup headingLevel={2} />}
          {!responding && !preview.sent_to && (
            <Actions>
              <Button
                variant="primary"
                label={
                  account && able ? fmt(w.respondAs, { name: account.display_name }) : w.respondNew
                }
                onPress={() => {
                  setFailure(null);
                  setResponding(true);
                }}
              />
            </Actions>
          )}
          <InvitationReport token={token} />
        </>
      )}
    </Screen>
  );
}
