import {
  PAYMENT_APPS,
  phoneAsTyped,
  usePaymentHandles,
  type PaymentApp,
  type PaymentHandlesApi,
} from '@yuppers/shared';
import { useEffect } from 'react';
import type { TextInputProps } from 'react-native';

import { announce } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { Actions, Button, Failure, Heading, Notice, P, TextField } from './ui';

/** The wording keys of each app's field, in `payments`. */
const FIELD = {
  venmo: { label: 'venmoLabel', hint: 'venmoHint', invalid: 'venmoInvalid' },
  cash_app: { label: 'cashAppLabel', hint: 'cashAppHint', invalid: 'cashAppInvalid' },
  paypal: { label: 'paypalLabel', hint: 'paypalHint', invalid: 'paypalInvalid' },
  zelle: { label: 'zelleLabel', hint: 'zelleHint', invalid: 'zelleInvalid' },
} as const;

/** The keyboard for each: none is a sign-in, and only Zelle may be an address. */
const KEYBOARD: Record<PaymentApp, TextInputProps['keyboardType']> = {
  venmo: 'default',
  cash_app: 'default',
  paypal: 'default',
  zelle: 'email-address',
};

interface Props {
  /** For tests: the service's calls. */
  client?: PaymentHandlesApi;
}

/**
 * Payment options on the account screen, as on the web
 * (`apps/web/src/components/PaymentHandles.tsx`): optional names in Venmo,
 * Cash App, PayPal and Zelle, saved encrypted, shown nowhere until the
 * person turns them on for a yup.
 */
export function PaymentHandles({ client = api }: Props) {
  const { wording } = useI18n();
  const w = wording.payments;
  const form = usePaymentHandles(client);
  const said = form.done === 'saved' ? w.saved : form.done === 'removed' ? w.removed : null;

  useEffect(() => {
    if (said) announce(said);
  }, [said]);

  const loading = form.saved === null && form.failure === null;
  const anySaved = PAYMENT_APPS.some((app) => Boolean(form.saved?.[app]));

  // A US number for Zelle is written the American way on leaving the field;
  // anything else is left as typed, and nothing changes if it already is.
  const showZelleNumber = () => {
    const shown = phoneAsTyped(form.inputs.zelle);
    if (shown !== form.inputs.zelle) form.set('zelle', shown);
  };

  return (
    <>
      <Heading level={2}>{w.heading}</Heading>
      <P>{w.intro}</P>
      {PAYMENT_APPS.map((app) => (
        <TextField
          key={app}
          testID={`payment-${app}`}
          label={w[FIELD[app].label]}
          hint={w[FIELD[app].hint]}
          error={form.invalid.includes(app) ? w[FIELD[app].invalid] : null}
          disabled={loading}
          keyboardType={KEYBOARD[app]}
          autoCapitalize="none"
          autoCorrect={false}
          autoComplete="off"
          spellCheck={false}
          value={form.inputs[app]}
          onChangeText={(value) => form.set(app, value)}
          onBlur={app === 'zelle' ? showZelleNumber : undefined}
        />
      ))}
      <Failure code={form.failure} />
      {said ? <Notice>{said}</Notice> : null}
      <Actions>
        <Button
          testID="payment-save"
          variant="primary"
          label={w.save}
          disabled={form.busy || form.saved === null}
          onPress={() => void form.save()}
        />
        {anySaved && (
          <Button
            testID="payment-remove"
            label={w.remove}
            disabled={form.busy}
            onPress={() => void form.remove()}
          />
        )}
      </Actions>
    </>
  );
}
