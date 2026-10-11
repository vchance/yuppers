import type { ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client';
import {
  applyTemplate,
  failureCode,
  pendingStart,
  type PendingStart,
  templateById,
  type IssuedInvitation,
  type RevisionSent,
} from '@yuppers/shared';
import * as Crypto from 'expo-crypto';
import { useFocusEffect, useRouter } from 'expo-router';
import { useCallback, useRef, useState } from 'react';

import { Actions, Button, Failure, Heading, P, Screen } from '../components/ui';
import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { deviceTimezone } from '../lib/time-zone';
import { Composer } from './Composer';
import { ExchangeView } from './ExchangeView';
import { SendInvitation } from './SendInvitation';

/** Loads one exchange, and again whenever its screen comes back to the front. */
function useExchange(id: string | null, pending?: PendingStart) {
  const [exchange, setExchange] = useState<Exchange | null>(pending?.exchange ?? null);
  const [failure, setFailure] = useState<ErrorCode | null>(null);

  const reload = useCallback(async () => {
    const at = pending?.created()?.id ?? id;
    if (!at) return null;
    try {
      const latest = await api.getExchange(at);
      setExchange(latest);
      setFailure(null);
      return latest;
    } catch (error) {
      setFailure(failureCode(error));
      return null;
    }
  }, [id, pending]);

  // Coming back from writing a counteroffer, the exchange shows it.
  useFocusEffect(
    useCallback(() => {
      // A fresh start has nothing to read, and one just made is in hand.
      if (!pending) void reload();
    }, [reload, pending]),
  );

  return { exchange, setExchange, failure, reload };
}

function Unavailable({ failure }: { failure: ErrorCode | null }) {
  const { wording } = useI18n();
  const router = useRouter();
  if (!failure) {
    return (
      <Screen>
        <P>{wording.common.loading}</P>
      </Screen>
    );
  }
  return (
    <Screen>
      <Heading>{wording.exchange.titleNoName}</Heading>
      <Failure code={failure} />
      <Actions>
        <Button label={wording.common.goHome} onPress={() => router.dismissTo('/')} />
      </Actions>
    </Screen>
  );
}

/**
 * One exchange. A draft is its composer; a first proposal just sent is the
 * step that sends its invitation link; anything further along is the
 * exchange view.
 */
export function ExchangeScreen({ id, pending }: { id: string | null; pending?: PendingStart }) {
  const router = useRouter();
  const { exchange, setExchange, failure, reload } = useExchange(id, pending);
  // Whether the first proposal has been sent from a fresh start: the screen
  // then stays as it is until its link has been dealt with.
  const sentFromFresh = useRef(false);
  // The invitation token, held only while this screen stays open: it is shown
  // once and cannot be fetched again.
  const [issued, setIssued] = useState<IssuedInvitation | null>(null);
  // Whether the step that sends the link is the screen: from sending a
  // first proposal until the person opens a way to send it and goes on, or
  // says they will send it later.
  const [sending, setSending] = useState(false);

  // The person opened a way to send the link. The service is told, so the
  // reminder on this screen and the chip in the list know; the screen
  // itself knows at once, and a request that fails changes nothing it can show.
  const shared = useCallback(() => {
    const at = new Date().toISOString();
    setExchange((current) => current && { ...current, invitation_shared_at: at });
    api.markInvitationShared(exchange?.id ?? id ?? '').catch(() => {});
  }, [id, exchange?.id, setExchange]);

  // The link is dealt with: a fresh start moves to the exchange's own screen.
  function finishSending() {
    setSending(false);
    if (pending && exchange) router.replace(`/exchanges/${exchange.id}`);
  }

  if (!exchange) return <Unavailable failure={failure} />;

  if (exchange.state === 'DRAFT') {
    return (
      <Composer
        exchange={exchange}
        reload={reload}
        onLeave={() => router.dismissTo('/')}
        pending={pending}
        onCreated={(made) => {
          setExchange((current) => (current?.id === made.id ? current : made));
          // The exchange has its own screen; this one, which had none, gives way.
          if (!sentFromFresh.current) router.replace(`/exchanges/${made.id}`);
        }}
        onSent={(sent: RevisionSent, boundTo: string | null) => {
          sentFromFresh.current = true;
          // A first proposal is not done until its link is sent: that step
          // comes next, as the screen, before the exchange itself is shown.
          const link = sent.invitation_token ? { token: sent.invitation_token, boundTo } : null;
          setIssued(link);
          setSending(link !== null);
          setExchange(sent.exchange);
        }}
      />
    );
  }

  if (sending && issued) {
    return (
      <SendInvitation
        exchange={exchange}
        issued={issued}
        onShared={shared}
        onDone={finishSending}
        onLater={finishSending}
      />
    );
  }

  return (
    <ExchangeView
      exchange={exchange}
      issued={issued}
      onIssued={setIssued}
      onShared={shared}
      onChange={setExchange}
      reload={reload}
    />
  );
}

/** Writing a counteroffer or an amendment, over the exchange it belongs to. */
export function ReviseScreen({ id }: { id: string }) {
  const router = useRouter();
  const { exchange, failure, reload } = useExchange(id);

  // Back to the exchange, which reloads as it comes to the front.
  const back = () => {
    if (router.canGoBack()) router.back();
    else router.replace(`/exchanges/${id}`);
  };

  if (!exchange) return <Unavailable failure={failure} />;
  return <Composer exchange={exchange} reload={reload} onLeave={back} onSent={back} />;
}

/**
 * A fresh start, written before the service has an exchange for it: a
 * template's id, or `blank`. The composer works on a local copy, and the
 * first change makes the exchange and moves to its screen, replacing this
 * one so that Back does not come to a screen with nothing behind it.
 */
export function NewDraftScreen({ from }: { from: string }) {
  const { wording } = useI18n();
  const { account } = useSession();
  const [pending] = useState(() => {
    const timezone = deviceTimezone();
    const template = from === 'blank' ? undefined : templateById(from);
    if (!template) return pendingStart(api, timezone, { kind: 'blank' }, null);
    const draft = applyTemplate(
      template,
      wording.templates.entries[template.id],
      account?.display_name ?? '',
      () => Crypto.randomUUID(),
    );
    return pendingStart(api, timezone, { kind: 'template', template }, draft);
  });
  return <ExchangeScreen id={null} pending={pending} />;
}
