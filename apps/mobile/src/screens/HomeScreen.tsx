import type { ErrorCode, ExchangeSummary } from '@yuppers/api-client';
import { failureCode, groupExchanges, invitationChip, labelText } from '@yuppers/shared';
import { useFocusEffect, useRouter } from 'expo-router';
import { useCallback, useState } from 'react';
import { Pressable, StyleSheet, View } from 'react-native';

import {
  Actions,
  Button,
  Card,
  Failure,
  Heading,
  Hint,
  P,
  Screen,
  Tag,
  Tags,
  Written,
} from '../components/ui';
import { CombinedNotice } from '../components/CombinedNotice';
import { ExampleCard } from '../components/ExampleCard';
import { Mark } from '../components/Mark';
import { NotificationsOffer } from '../components/Notifications';
import { StatusChip } from '../components/StatusChip';
import { useI18n } from '../lib/context';
import { forgetInvitation } from '../lib/invitation';
import { api } from '../lib/session';
import { space, TOUCH_TARGET } from '../lib/theme';

/**
 * The signed-in person's exchanges, the way to start one, and the way to
 * open an invitation that did not open by itself. What is in progress comes
 * first, since that is what may be waiting on them; drafts they never sent
 * come next; what is closed is kept but folded away, so it never buries the
 * rest. Within a group, most recently changed first.
 *
 * With no exchanges yet, the screen is the mark, what to do, and the ways to
 * start, in one place rather than buttons over an empty list.
 */
export function HomeScreen() {
  const { wording, fmt } = useI18n();
  const router = useRouter();
  const w = wording.home;

  const [exchanges, setExchanges] = useState<ExchangeSummary[] | null>(null);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [showClosed, setShowClosed] = useState(false);

  const load = useCallback(async () => {
    try {
      setExchanges(await api.listExchanges());
      setFailure(null);
    } catch (error) {
      setFailure(failureCode(error));
    }
  }, []);

  // Coming back from an exchange, the list shows what changed there.
  useFocusEffect(
    useCallback(() => {
      void load();
    }, [load]),
  );

  // Starting is a choice of how to begin (`StartScreen`), then the composer.
  const start = () => router.push('/new');

  const groups = exchanges ? groupExchanges(exchanges) : null;
  const empty = exchanges?.length === 0;
  const actions = (
    <Actions>
      <Button variant="primary" label={w.start} onPress={start} />
      <Button
        label={wording.mobile.openInvitation.title}
        onPress={() => {
          // Asking from here means a new one: an invitation looked at earlier is let go.
          forgetInvitation();
          router.push('/invitation');
        }}
      />
    </Actions>
  );

  return (
    <Screen
      refreshing={refreshing}
      onRefresh={() => {
        setRefreshing(true);
        void load().finally(() => setRefreshing(false));
      }}>
      <Heading>{w.title}</Heading>
      <CombinedNotice />
      {empty ? null : actions}
      <Failure code={failure} />
      <NotificationsOffer exchanges={exchanges} />

      {!exchanges && !failure && <P>{wording.common.loading}</P>}
      {empty ? (
        <Card style={styles.empty}>
          <Mark width={96} />
          <P style={styles.emptyText}>{w.empty}</P>
          <ExampleCard />
          <View style={styles.emptyActions}>{actions}</View>
        </Card>
      ) : null}
      {groups && (
        <>
          <Group heading={w.groupOpen} exchanges={groups.open} />
          <Group heading={w.groupDrafts} exchanges={groups.drafts} />
          {groups.closed.length > 0 && (
            <>
              <Heading level={2}>{w.groupClosed}</Heading>
              <Actions>
                <Button
                  label={
                    showClosed ? w.hideClosed : fmt(w.showClosed, { count: groups.closed.length })
                  }
                  expanded={showClosed}
                  onPress={() => setShowClosed((shown) => !shown)}
                />
              </Actions>
              {showClosed && <Cards exchanges={groups.closed} />}
            </>
          )}
        </>
      )}
    </Screen>
  );
}

/**
 * On the first screen, before signing in: the way to an invitation. Someone
 * invited is usually new and signed out; this takes them to the same screen a
 * link opens, which asks them to sign in and then shows the proposal
 * (DESIGN.md §8, §9).
 */
export function InvitedEntry() {
  const { wording } = useI18n();
  const router = useRouter();
  const w = wording.mobile;
  return (
    <Card>
      <Heading level={2}>{w.invited.heading}</Heading>
      <P>{w.invited.introSignIn}</P>
      <Actions>
        <Button
          label={w.openInvitation.title}
          onPress={() => {
            // Asking from here means a new one: an invitation looked at earlier is let go.
            forgetInvitation();
            router.push('/invitation');
          }}
        />
        {/* A newcomer can see what a yup is before signing in (DESIGN.md §4.3). */}
        <Button
          variant="link"
          label={wording.sample.signInLine}
          onPress={() => router.push('/example')}
        />
      </Actions>
    </Card>
  );
}

function Group({ heading, exchanges }: { heading: string; exchanges: readonly ExchangeSummary[] }) {
  if (exchanges.length === 0) return null;
  return (
    <>
      <Heading level={2}>{heading}</Heading>
      <Cards exchanges={exchanges} />
    </>
  );
}

function Cards({ exchanges }: { exchanges: readonly ExchangeSummary[] }) {
  const { wording, fmt, moment } = useI18n();
  const router = useRouter();
  const w = wording.home;
  return exchanges.map((exchange) => {
    const state = exchange.closed_outcome
      ? wording.outcomes[exchange.closed_outcome]
      : wording.states[exchange.state];
    const title = exchange.other_party_name
      ? fmt(w.withParty, { name: exchange.other_party_name })
      : w.noParty;
    const reference = fmt(w.reference, { code: exchange.display_code });
    const updated = fmt(w.updated, { date: moment(exchange.updated_at) });
    // The name is the other party's own words, and could be written to read
    // like a state or a reference. In the spoken label the product's facts
    // come first and the name last, on one line and without characters that
    // change the direction of text.
    const spokenTitle = exchange.other_party_name
      ? fmt(w.withParty, { name: labelText(exchange.other_party_name) })
      : w.noParty;
    // The initiator holds the link that brings the other party in, and
    // Yuppers never sends it: until someone joins, the card says whether they
    // have sent it (`invitationChip`), on a chip like an item's status.
    const chip = invitationChip(exchange);
    const chipText =
      chip === 'notSent'
        ? w.notSent
        : chip === 'waiting'
          ? fmt(w.waitingFor, { name: labelText(exchange.other_party_name) })
          : null;
    return (
      // One button per exchange, read as one: where it stands, its reference
      // and when it changed, then who it is with, then that it opens.
      <Pressable
        key={exchange.id}
        accessibilityRole="button"
        accessibilityLabel={[state, chipText, reference, updated, spokenTitle]
          .filter(Boolean)
          .join('. ')}
        accessibilityHint={wording.a11y.openExchange}
        onPress={() => router.push(`/exchanges/${exchange.id}`)}
        style={({ pressed }) => [styles.row, pressed && styles.pressed]}>
        <Card>
          {exchange.other_party_name ? <Written>{title}</Written> : <P>{title}</P>}
          <Tags>
            <Tag>{state}</Tag>
            {chipText && (
              <StatusChip status={chip === 'notSent' ? 'PENDING' : 'CLAIMED'}>{chipText}</StatusChip>
            )}
          </Tags>
          <Hint>{reference}</Hint>
          <Hint>{updated}</Hint>
        </Card>
      </Pressable>
    );
  });
}

const styles = StyleSheet.create({
  row: { minHeight: TOUCH_TARGET },
  pressed: { opacity: 0.6 },
  empty: { alignItems: 'center', paddingVertical: space.xl, gap: space.l },
  emptyText: { textAlign: 'center', maxWidth: 400 },
  // The buttons wrap as a row does, centred under the words.
  emptyActions: { alignItems: 'center', maxWidth: '100%' },
});
