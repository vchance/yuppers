import type { ErrorCode, ExchangeSummary } from '@yuppers/api-client';
import {
  beginYup,
  draftFromCopy,
  failureCode,
  fractionDigitsOf,
  labelText,
  TEMPLATES,
  type StartChoice,
  type Template,
} from '@yuppers/shared';
import * as Crypto from 'expo-crypto';
import { useRouter } from 'expo-router';
import { useEffect, useState } from 'react';

import {
  Actions,
  Button,
  Card,
  Choice,
  ErrorNote,
  Failure,
  Heading,
  Hint,
  Notice,
  P,
  Screen,
  Tag,
  Tags,
  Written,
} from '../components/ui';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { deviceTimezone } from '../lib/time-zone';

type CopyDraft = Parameters<typeof beginYup>[3];

/**
 * The first step of a new yup (DESIGN.md §4.4): a short list of common
 * agreements, then the blank form and copying an earlier yup. Choosing makes
 * the draft and opens it in the composer; nothing else is asked here. The
 * same list, from the same wording, as on the web.
 */
export function StartScreen() {
  const { wording } = useI18n();
  const router = useRouter();
  const w = wording.templates;

  const [mode, setMode] = useState<'choose' | 'copy'>('choose');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [tooMany, setTooMany] = useState(false);

  /** Makes the draft with the copy of an earlier yup, and opens it. */
  async function begin(choice: StartChoice, draft: CopyDraft) {
    setBusy(true);
    setFailure(null);
    setTooMany(false);
    try {
      const exchange = await beginYup(api, deviceTimezone(), choice, draft);
      router.replace(`/exchanges/${exchange.id}`);
    } catch (error) {
      const code = failureCode(error);
      // Here the limit is on exchanges started today, not on codes.
      if (code === 'TOO_MANY_REQUESTS') setTooMany(true);
      else setFailure(code);
      setBusy(false);
    }
  }

  // A template or the blank form opens the composer with a working copy of
  // its own and no exchange on the service: that is made when something is
  // first changed.
  function startFrom(template: Template) {
    router.replace(`/new/${template.id}`);
  }

  return (
    <Screen>
      <Heading>{mode === 'copy' ? w.copyHeading : w.chooserTitle}</Heading>
      <Failure code={failure} />
      {tooMany && <ErrorNote>{wording.home.tooManyToday}</ErrorNote>}

      {mode === 'choose' ? (
        <>
          <Heading level={2}>{w.chooserHeading}</Heading>
          <P>{w.chooserIntro}</P>
          {TEMPLATES.map((template) => {
            const entry = w.entries[template.id];
            return (
              <Card key={template.id}>
                <Button
                  label={entry.name}
                  hint={entry.summary}
                  disabled={busy}
                  onPress={() => startFrom(template)}
                />
                <Hint>{entry.summary}</Hint>
                {entry.warning && <Notice tone="warning" quiet>{entry.warning}</Notice>}
              </Card>
            );
          })}

          <Heading level={2}>{w.orHeading}</Heading>
          <Card>
            <Button
              label={w.blank.name}
              hint={w.blank.summary}
              disabled={busy}
              onPress={() => router.replace('/new/blank')}
            />
            <Hint>{w.blank.summary}</Hint>
          </Card>
          <Card>
            <Button
              label={w.copy.name}
              hint={w.copy.summary}
              disabled={busy}
              onPress={() => setMode('copy')}
            />
            <Hint>{w.copy.summary}</Hint>
          </Card>

          <Heading level={2}>{w.notForHeading}</Heading>
          <P>{w.notFor}</P>
          <P>{w.notForAdvice}</P>
        </>
      ) : (
        <CopyPrevious
          busy={busy}
          onBack={() => setMode('choose')}
          onCopy={(draft) => void begin({ kind: 'copy' }, draft)}
          onFailure={setFailure}
        />
      )}
    </Screen>
  );
}

/**
 * Copying an earlier yup: the person's yups, newest first, closed ones
 * included, then who the copy is for. The other party is asked about every
 * time, and the answer starts as "someone else", so that a name never
 * reaches the wrong person by default (DESIGN.md §4.4).
 */
function CopyPrevious({
  busy,
  onBack,
  onCopy,
  onFailure,
}: {
  busy: boolean;
  onBack(): void;
  onCopy(draft: CopyDraft): void;
  onFailure(code: ErrorCode | null): void;
}) {
  const { wording, fmt, moment } = useI18n();
  const w = wording.templates;
  const [yups, setYups] = useState<ExchangeSummary[] | null>(null);
  const [chosen, setChosen] = useState<ExchangeSummary | null>(null);
  const [forSame, setForSame] = useState<'else' | 'same'>('else');
  const [reading, setReading] = useState(false);
  const [noTerms, setNoTerms] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api.listExchanges().then(
      (found) => {
        // A draft that was never sent has no terms to copy.
        if (!cancelled) setYups(found.filter((yup) => yup.state !== 'DRAFT'));
      },
      (error: unknown) => {
        if (!cancelled) onFailure(failureCode(error));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [onFailure]);

  async function copy(yup: ExchangeSummary) {
    setReading(true);
    setNoTerms(false);
    onFailure(null);
    try {
      const exchange = await api.getExchange(yup.id);
      // The last version: in force, or else the last one sent.
      const last = exchange.in_force_revision ?? exchange.open_revision ?? null;
      if (!last) {
        setNoTerms(true);
        setReading(false);
        return;
      }
      onCopy(
        draftFromCopy(
          last.terms,
          exchange.you,
          forSame === 'same',
          fractionDigitsOf(exchange.currency),
          () => Crypto.randomUUID(),
        ),
      );
    } catch (error) {
      onFailure(failureCode(error));
      setReading(false);
    }
  }

  if (chosen) {
    const name = chosen.other_party_name;
    return (
      <>
        <P>{w.copyIntro}</P>
        <Choice<'else' | 'same'>
          label={w.copyForLegend}
          value={forSame}
          options={[
            { value: 'else', label: w.copyForSomeoneElse },
            {
              value: 'same',
              label: name ? fmt(w.copyForSame, { name: labelText(name) }) : w.copyForSameNoName,
            },
          ]}
          onChange={setForSame}
        />
        {noTerms && <ErrorNote>{w.copyNoTerms}</ErrorNote>}
        <Actions>
          <Button
            variant="primary"
            label={busy || reading ? w.starting : w.copyStart}
            disabled={busy || reading}
            onPress={() => void copy(chosen)}
          />
          <Button label={w.back} disabled={busy || reading} onPress={() => setChosen(null)} />
        </Actions>
      </>
    );
  }

  return (
    <>
      <P>{w.copyIntro}</P>
      {!yups && <P>{w.copyLoading}</P>}
      {yups?.length === 0 && <P>{w.copyNone}</P>}
      {yups?.map((yup) => (
        <Card key={yup.id}>
          {yup.other_party_name ? (
            <Written>{fmt(wording.home.withParty, { name: yup.other_party_name })}</Written>
          ) : (
            <P>{wording.home.noParty}</P>
          )}
          <Tags>
            <Tag>
              {yup.closed_outcome ? wording.outcomes[yup.closed_outcome] : wording.states[yup.state]}
            </Tag>
          </Tags>
          <Hint>{fmt(wording.home.reference, { code: yup.display_code })}</Hint>
          <Hint>{fmt(wording.home.updated, { date: moment(yup.updated_at) })}</Hint>
          <Actions>
            <Button
              label={w.copyStart}
              accessibilityLabel={
                yup.other_party_name
                  ? fmt(w.copyStartNamed, { name: labelText(yup.other_party_name) })
                  : undefined
              }
              disabled={busy}
              onPress={() => {
                setForSame('else');
                setChosen(yup);
              }}
            />
          </Actions>
        </Card>
      ))}
      <Actions>
        <Button label={w.back} onPress={onBack} />
      </Actions>
    </>
  );
}
