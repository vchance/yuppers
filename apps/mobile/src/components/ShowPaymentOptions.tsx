import type { ExchangeView as Exchange, RevisionTerms } from '@yuppers/api-client';
import {
  receivesMoney,
  showOffered,
  useHasPaymentHandles,
  useShowPaymentOptions,
  type PaymentOptionsApi,
} from '@yuppers/shared';
import { useRouter } from 'expo-router';
import { useCallback, useEffect } from 'react';

import { announce } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { useShownAgain } from './PaymentOptionsRow';
import { Actions, Button, Card, Check, Failure, Heading, Hint, Notice, P } from './ui';

interface Props {
  exchange: Exchange;
  otherName: string;
  reload(): Promise<unknown>;
  /** For tests: the service's calls. */
  client?: PaymentOptionsApi;
}

/**
 * "Your payment options" on a yup, as on the web
 * (`apps/web/src/components/ShowPaymentOptions.tsx`): for a party who
 * receives money, a switch, off until turned on, that shows their saved
 * payment options to the other party while they owe money here. Not part
 * of the agreement; turning it off takes them away at once.
 */
export function ShowPaymentOptions({ exchange, otherName, reload, client = api }: Props) {
  const { wording, fmt } = useI18n();
  const router = useRouter();
  const w = wording.payments;
  const refresh = useCallback(() => void reload(), [reload]);
  // Read again on coming back, as from adding some on the payment options screen.
  const control = useShowPaymentOptions(client, exchange, refresh, useShownAgain());
  const said =
    control.changed === 'shown'
      ? fmt(w.shownNow, { name: otherName })
      : control.changed === 'hidden'
        ? w.hiddenNow
        : null;

  useEffect(() => {
    if (said) announce(said);
  }, [said]);

  if (!showOffered(exchange) || control.hasHandles === null) return null;

  return (
    <Card>
      <Heading level={2}>{w.showHeading}</Heading>
      {control.hasHandles || control.shown ? (
        <>
          <Check
            testID="show-payment-options"
            label={w.showLabel}
            hint={fmt(w.showHint, { name: otherName })}
            value={control.shown}
            disabled={control.busy}
            onChange={(on) => void control.set(on)}
          />
          <Hint>{fmt(w.showHint, { name: otherName })}</Hint>
          <Failure code={control.failure} />
          {said ? <Notice>{said}</Notice> : null}
          <Actions>
            <Button
              variant="link"
              label={w.manage}
              onPress={() => router.push('/account/payments')}
            />
          </Actions>
        </>
      ) : (
        <>
          <P>{w.noneSaved}</P>
          <Actions>
            <Button variant="link" label={w.addInAccount} onPress={() => router.push('/account/payments')} />
          </Actions>
        </>
      )}
    </Card>
  );
}

interface WhenSigningProps {
  terms: RevisionTerms;
  you: string;
  shown: boolean;
  value: boolean;
  onChange(value: boolean): void;
  client?: Pick<PaymentOptionsApi, 'paymentHandles'>;
}

/**
 * Beside signing or sending terms in which the person receives money: a
 * switch, off, to show their payment options on this yup too. Only for
 * someone who has saved some and does not show them here yet. Not part of
 * what is signed: it is set once the signature has gone.
 */
export function ShowWhenSigning({ terms, you, shown, value, onChange, client = api }: WhenSigningProps) {
  const { wording } = useI18n();
  const has = useHasPaymentHandles(client);
  if (!has || shown || !receivesMoney(terms, you)) return null;
  return (
    <Check
      testID="show-when-signing"
      label={wording.payments.showWhenSigning}
      value={value}
      onChange={onChange}
    />
  );
}
