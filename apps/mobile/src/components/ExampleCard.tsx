import { useRouter } from 'expo-router';
import { StyleSheet } from 'react-native';

import { useI18n } from '../lib/context';
import { space } from '../lib/theme';
import { Actions, Button, Card, Heading, Hint, P, Tag, Tags } from './ui';

/**
 * The card on the empty home screen that shows what a yup looks like and
 * opens the sample (DESIGN.md §4.3). It is described by its words, not by
 * colour, and it is only drawn where there are no yups of the person's own.
 */
export function ExampleCard() {
  const { wording, fmt } = useI18n();
  const router = useRouter();
  const w = wording.sample;
  return (
    <Card style={styles.card}>
      <Heading level={2}>{w.homeHeading}</Heading>
      <P>{w.homeBody}</P>
      <P>{fmt(w.cardWith, { name: w.partyB })}</P>
      <Tags>
        <Tag>{w.cardState}</Tag>
      </Tags>
      <Hint>{w.cardDetail}</Hint>
      <Actions>
        <Button label={w.see} onPress={() => router.push('/example')} />
      </Actions>
    </Card>
  );
}

const styles = StyleSheet.create({
  card: { gap: space.s },
});
