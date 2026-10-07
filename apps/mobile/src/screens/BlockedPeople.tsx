import { labelText, useBlockedPeople, type BlockedPerson } from '@yuppers/shared';
import { useFocusEffect, useRouter } from 'expo-router';
import { useCallback } from 'react';
import { Pressable, StyleSheet, Text } from 'react-native';

import { Actions, Button, Card, Failure, Heading, Hint, Notice, P } from '../components/ui';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { space, TOUCH_TARGET, type, useColors } from '../lib/theme';
import { LeftExchangeNote } from './Claimant';

/**
 * The people this account has blocked, and the way to unblock each
 * (DESIGN.md §9). A person is shown as an exchange shared with them names
 * them, with that exchange's reference; the app never learns who they are
 * beyond that.
 */
export function BlockedPeople() {
  const { wording, fmt, moment } = useI18n();
  const router = useRouter();
  const colors = useColors();
  const w = wording.safety;
  const { people, failure, busy, unblocked, load, unblock } = useBlockedPeople(api);

  // Read again each time the screen comes back to the front: someone may have
  // been blocked or unblocked from an exchange in the meantime.
  useFocusEffect(
    useCallback(() => {
      load();
    }, [load]),
  );

  const nameOf = (person: BlockedPerson) => person.name || wording.party.other;

  return (
    <>
      <Heading level={2}>{w.blockedHeading}</Heading>
      <Failure code={failure} />
      {/* Read out, because the button that was pressed leaves with its entry. */}
      {unblocked !== null && <Notice>{fmt(w.unblocked, { name: unblocked })}</Notice>}

      {!people && !failure && <P>{wording.common.loading}</P>}
      {people?.length === 0 && <P>{w.blockedEmpty}</P>}
      {people && people.length > 0 && (
        <>
          <P>{w.blockedIntro}</P>
          {people.map((person) => {
            const reference = fmt(wording.home.reference, { code: person.display_code });
            return (
              <Card key={person.exchange_id}>
                {/* Their name, as written, leads to the exchange that names them,
                    unless the reader has left it: then it only names them. */}
                <Pressable
                  accessibilityRole={person.left ? 'text' : 'link'}
                  accessibilityLabel={labelText(nameOf(person))}
                  accessibilityHint={reference}
                  disabled={person.left === true}
                  onPress={() => router.push(`/exchanges/${person.exchange_id}`)}
                  style={({ pressed }) => [styles.link, pressed && styles.pressed]}>
                  <Text
                    style={[
                      type.body,
                      styles.name,
                      { color: colors.link, borderStartColor: colors.border },
                    ]}>
                    {nameOf(person)}
                  </Text>
                </Pressable>
                <Hint>{reference}</Hint>
                {person.left && <LeftExchangeNote />}
                <Hint>{fmt(w.blockedSince, { date: moment(person.blocked_at) })}</Hint>
                <Actions>
                  <Button
                    label={fmt(w.unblock, { name: nameOf(person) })}
                    disabled={busy !== null}
                    onPress={() => unblock(person, nameOf(person))}
                  />
                </Actions>
              </Card>
            );
          })}
        </>
      )}
    </>
  );
}

const styles = StyleSheet.create({
  link: { minHeight: TOUCH_TARGET, justifyContent: 'center' },
  pressed: { opacity: 0.6 },
  // Set apart as the person's own words, like every name they wrote.
  name: { borderStartWidth: 3, paddingStart: space.m, textDecorationLine: 'underline' },
});
