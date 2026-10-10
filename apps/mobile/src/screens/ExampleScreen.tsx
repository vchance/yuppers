import type { components } from '@yuppers/api-client';
import { moveWording, sampleYup, statusWording } from '@yuppers/shared';
import { useRouter } from 'expo-router';
import { useMemo } from 'react';

import { EventList } from '../components/EventList';
import { RecordSummary } from '../components/RecordSummary';
import { TermsView } from '../components/TermsView';
import { Actions, Button, Card, Heading, Hint, Notice, P, Screen, Tag, Tags } from '../components/ui';
import { useI18n, useSession } from '../lib/context';
import { deviceTimezone } from '../lib/time-zone';

type Contribution = components['schemas']['ContributionDto'];

/**
 * The sample yup (DESIGN.md §4.3): a worked example, reachable signed out. It
 * is drawn by the components that draw real yups, from a fixed document with
 * no service behind it, so nothing here is created, saved or sent. Every
 * action is shown disabled, with its own name and a line saying why it does
 * nothing, and a banner says in words, not by colour, that it is an example.
 */
export function ExampleScreen() {
  const { wording, fmt, moment, language } = useI18n();
  const { account } = useSession();
  const router = useRouter();
  const w = wording.sample;

  const sample = useMemo(
    () => sampleYup(new Date(), deviceTimezone(), w, language),
    [w, language],
  );

  function actionsFor(item: Contribution) {
    const money = sample.money.has(item.id);
    const status = sample.statuses.get(item.id) ?? 'PENDING';
    const provider = item.from === 'A' ? w.firstNameA : w.firstNameB;
    // The deposit is done. The others show the step that comes next, disabled.
    const next = status === 'PENDING' ? 'CLAIM' : null;
    return (
      <>
        <P>{statusWording(wording, status, money)}</P>
        {next && (
          <>
            <Actions>
              <Button
                label={moveWording(wording, next, money)}
                disabled
                hint={w.actionsOff}
                onPress={() => {}}
              />
            </Actions>
            <Hint>
              {`${w.actionsOff} ${
                item.id === sample.terms.contributions[2].id
                  ? w.waitsOn
                  : fmt(w.wouldTap, { name: provider })
              }`}
            </Hint>
          </>
        )}
      </>
    );
  }

  return (
    <Screen>
      <Heading>{w.title}</Heading>
      <Notice tone="warning">{w.banner}</Notice>
      <Tags>
        <Tag>{wording.states.ACTIVE}</Tag>
      </Tags>

      <Heading level={2}>{w.termsHeading}</Heading>
      <Card>
        <TermsView
          terms={sample.terms}
          currency={sample.currency}
          timezone={sample.timezone}
          you={null}
          statuses={sample.statuses}
          footer={actionsFor}
        />
      </Card>

      <RecordSummary record={sample.record} />

      <Heading level={2}>{w.historyHeading}</Heading>
      <EventList
        events={sample.events}
        parties={sample.parties}
        reader={null}
        when={moment}
        money={sample.money}
      />
      <Hint>{w.fingerprint}</Hint>

      <Actions>
        <Button
          variant="primary"
          label={account ? w.startOwn : w.signInToStart}
          onPress={() => router.dismissTo('/')}
        />
      </Actions>
    </Screen>
  );
}
