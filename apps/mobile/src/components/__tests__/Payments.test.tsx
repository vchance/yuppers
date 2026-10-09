import type { ExchangeView } from '@yuppers/api-client';
import {
  ApiFailure,
  createI18n,
  wordingFor,
  type PaymentHandles as Handles,
} from '@yuppers/shared';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import * as Clipboard from 'expo-clipboard';
import type { ReactNode } from 'react';
import { SafeAreaProvider } from 'react-native-safe-area-context';

import { I18nContext } from '../../lib/context';
import { PaymentOptionsScreen } from '../../screens/PaymentOptionsScreen';
import { PaymentOptionsRow } from '../PaymentOptionsRow';
import { PaySheet } from '../PaySheet';
import { ShowPaymentOptions } from '../ShowPaymentOptions';

/*
 * Payment options (`payments.ts`): adding, editing and removing them
 * one app at a time on their own screen, reached from the account;
 * showing them on a yup; and the payer's sheet, whose buttons open each
 * app by its web address and record nothing; "I've paid" stays the payer's own.
 */

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn(async () => true) }));
const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush }),
  useFocusEffect: () => {},
}));

const wording = wordingFor('en');
const w = wording.payments;
const PAYMENT = '22222222-2222-4222-8222-222222222222';

const DANA: Handles = {
  venmo: 'dana-fixes',
  cash_app: 'DanaFixes',
  paypal: 'DanaFixes',
  zelle: '+12025550142',
};
const NONE: Handles = { venmo: null, cash_app: null, paypal: null, zelle: null };

const NO_CHANGES = { venmo: null, cash_app: null, paypal: null, zelle: null };

function exchange(
  theirs: Handles | null,
  shown = false,
  theirsChanged: Record<keyof Handles, string | null> = NO_CHANGES,
): ExchangeView {
  return {
    id: '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70',
    version: 4,
    state: 'ACTIVE',
    you: 'B',
    counterparty: 'CONFIRMED',
    currency: 'USD',
    timezone: 'America/Chicago',
    display_code: 'DSPT-8M3R',
    in_force_revision: {
      id: 'c0000000-0000-4000-8000-000000000001',
      sequence: 1,
      author: 'A',
      accepted_by: ['A', 'B'],
      content_hash: 'ab'.repeat(32),
      expires_at: '2026-10-16T12:00:00Z',
      terms: {
        party_a_name: 'Ana Ruiz',
        party_b_name: 'Ben Ortiz',
        terms: 'Repair the back fence.',
        contributions: [
          {
            id: PAYMENT,
            from: 'B',
            type: 'MONEY',
            description: 'Rent & deposit #3',
            due: { kind: 'ON_AGREEMENT' },
            required: true,
            amount_minor: 10050,
          },
        ],
      },
    },
    contributions: [{ id: PAYMENT, status: 'PENDING' }],
    payment_options: { shown, theirs, theirs_changed: theirsChanged },
  };
}

function wrap(children: ReactNode) {
  return (
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 390, height: 844 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}>
      <I18nContext.Provider value={createI18n('en', wording, () => {})}>{children}</I18nContext.Provider>
    </SafeAreaProvider>
  );
}

afterEach(() => jest.clearAllMocks());

test('the sheet offers text-only buttons that open each app, prefilled, and Zelle to copy', async () => {
  const view = exchange(DANA);
  const contribution = view.in_force_revision!.terms.contributions[0];
  const opened: string[] = [];
  const onPaid = jest.fn();
  const onClose = jest.fn();
  await render(
    wrap(
      <PaySheet
        exchange={view}
        contribution={contribution}
        otherName="Ana Ruiz"
        onClose={onClose}
        onPaid={onPaid}
        client={{ getExchange: async () => view }}
        open={(url) => opened.push(url)}
      />,
    ),
  );

  expect(await screen.findByRole('header', { name: 'Pay Ana Ruiz $100.50' })).toBeTruthy();
  expect(
    screen.getByText(
      'Added by Ana Ruiz. Yuppers doesn’t check it or move money. Make sure it’s the right person.',
    ),
  ).toBeTruthy();
  expect(screen.getByText(`${wording.terms.moneyOutside} ${w.appTerms}`)).toBeTruthy();
  expect(screen.getByTestId('pay-sheet').props.accessibilityViewIsModal).toBe(true);

  // Buttons named by the app, in words; each says whose it is and what it fills in.
  const venmo = screen.getByRole('button', { name: w.open.venmo });
  expect(venmo.props.accessibilityHint).toBe(
    'Venmo @dana-fixes. Opens with $100.50 and the note filled in. Check both before you send.',
  );
  await fireEvent.press(venmo);
  await fireEvent.press(screen.getByRole('button', { name: w.open.cash_app }));
  await fireEvent.press(screen.getByRole('button', { name: w.open.paypal }));
  expect(opened).toEqual([
    'https://venmo.com/u/dana-fixes?txn=pay&amount=100.50&note=Rent%20%26%20deposit%20%233%20%C2%B7%20yup%20DSPT-8M3R',
    'https://cash.app/$DanaFixes/100.50',
    'https://www.paypal.me/DanaFixes/100.50USD',
  ]);
  // Opening a link records nothing.
  expect(onPaid).not.toHaveBeenCalled();

  // Zelle: no link; the number, and the amount, to copy.
  expect(screen.getByText(w.zelleNoLinks)).toBeTruthy();
  expect(screen.getByText('(202) 555-0142')).toBeTruthy();
  await fireEvent.press(screen.getByRole('button', { name: 'Copy Zelle' }));
  await fireEvent.press(screen.getByRole('button', { name: 'Copy Amount' }));
  expect(Clipboard.setStringAsync).toHaveBeenCalledWith('(202) 555-0142');
  expect(Clipboard.setStringAsync).toHaveBeenCalledWith('100.50');

  // "I've paid" hands over to the claim, which the payer still sends.
  await fireEvent.press(screen.getByRole('button', { name: wording.exchange.moneyMoves.CLAIM }));
  expect(onPaid).toHaveBeenCalled();
});

test('options the payee stopped showing are not offered once the sheet opens', async () => {
  const view = exchange(DANA);
  const opened: string[] = [];
  await render(
    wrap(
      <PaySheet
        exchange={view}
        contribution={view.in_force_revision!.terms.contributions[0]}
        otherName="Ana Ruiz"
        onClose={() => {}}
        onPaid={() => {}}
        client={{ getExchange: async () => exchange(null) }}
        open={(url) => opened.push(url)}
      />,
    ),
  );
  expect(
    await screen.findByText(
      'Ana Ruiz no longer shows payment options here. Pay any way the two of you arrange.',
    ),
  ).toBeTruthy();
  expect(screen.queryByRole('button', { name: w.open.venmo })).toBeNull();
});

/** A stand-in for the service's payment option calls, one app at a time. */
function handlesClient(start: Handles = NONE) {
  const client = {
    saved: { ...start },
    paymentHandles: jest.fn(async () => client.saved),
    setPaymentHandle: jest.fn(async (kind: keyof Handles, value: string) => {
      client.saved = { ...client.saved, [kind]: value };
      return client.saved;
    }),
    removePaymentHandle: jest.fn(async (kind: keyof Handles) => {
      client.saved = { ...client.saved, [kind]: null };
      return client.saved;
    }),
  };
  return client;
}

const fill = (message: string, app: string) => message.replace('{app}', app);

test('the account shows one row with the apps added, that opens the payment options screen', async () => {
  const client = handlesClient({ ...NONE, venmo: 'ana-pays', zelle: '+12025550142' });
  await render(wrap(<PaymentOptionsRow client={client} />));
  const row = await screen.findByRole('button', { name: `${w.heading}, Venmo, Zelle` });
  await fireEvent.press(row);
  expect(mockPush).toHaveBeenCalledWith('/account/payments');
});

test('with none added, the row says so', async () => {
  await render(wrap(<PaymentOptionsRow client={handlesClient()} />));
  expect(await screen.findByRole('button', { name: `${w.heading}, ${w.noneAdded}` })).toBeTruthy();
});

test('payment options are added one app at a time, edited, and removed after asking', async () => {
  const client = handlesClient();
  await render(wrap(<PaymentOptionsScreen client={client} />));
  // What they are for, with none added.
  expect(await screen.findByText(w.empty)).toBeTruthy();
  expect(screen.getByText(w.intro)).toBeTruthy();

  // Venmo: chosen first, from all four, then its one field.
  await fireEvent.press(screen.getByRole('button', { name: w.add }));
  expect(await screen.findByRole('header', { name: w.pickHeading })).toBeTruthy();
  for (const app of ['Venmo', 'Cash App', 'PayPal', 'Zelle']) {
    expect(screen.getByRole('button', { name: app })).toBeTruthy();
  }
  await fireEvent.press(screen.getByRole('button', { name: 'Venmo' }));
  expect(await screen.findByRole('header', { name: fill(w.addHeading, 'Venmo') })).toBeTruthy();
  expect(screen.queryByLabelText(w.zelleLabel)).toBeNull();
  // Not one: said, and nothing sent.
  await fireEvent.changeText(screen.getByLabelText(w.venmoLabel), 'ana');
  await fireEvent.press(screen.getByRole('button', { name: w.saveOne }));
  expect(await screen.findByText(w.venmoInvalid)).toBeTruthy();
  expect(client.setPaymentHandle).not.toHaveBeenCalled();
  await fireEvent.changeText(screen.getByLabelText(w.venmoLabel), '@ana-pays');
  await fireEvent.press(screen.getByRole('button', { name: w.saveOne }));
  expect(await screen.findByText(fill(w.savedOne, 'Venmo'))).toBeTruthy();
  expect(client.setPaymentHandle).toHaveBeenLastCalledWith('venmo', 'ana-pays');

  // Zelle: Venmo is no longer offered; a US number written the American way.
  await fireEvent.press(screen.getByRole('button', { name: w.add }));
  await screen.findByRole('header', { name: w.pickHeading });
  expect(screen.queryByRole('button', { name: 'Venmo' })).toBeNull();
  await fireEvent.press(screen.getByRole('button', { name: 'Zelle' }));
  await fireEvent.changeText(screen.getByLabelText(w.zelleLabel), '202-555-0142');
  await fireEvent(screen.getByLabelText(w.zelleLabel), 'blur');
  expect(screen.getByLabelText(w.zelleLabel).props.value).toBe('(202) 555-0142');
  await fireEvent.press(screen.getByRole('button', { name: w.saveOne }));
  expect(await screen.findByText('(202) 555-0142')).toBeTruthy();
  expect(client.setPaymentHandle).toHaveBeenLastCalledWith('zelle', '+12025550142');
  expect(screen.getByRole('header', { name: 'Venmo' })).toBeTruthy();
  expect(screen.getByRole('header', { name: 'Zelle' })).toBeTruthy();

  // Edit: the same field, filled in.
  await fireEvent.press(screen.getByRole('button', { name: fill(w.editWhat, 'Venmo') }));
  expect(await screen.findByRole('header', { name: fill(w.editHeading, 'Venmo') })).toBeTruthy();
  expect(screen.getByLabelText(w.venmoLabel).props.value).toBe('ana-pays');
  await fireEvent.changeText(screen.getByLabelText(w.venmoLabel), 'ana-fixes');
  await fireEvent.press(screen.getByRole('button', { name: w.saveOne }));
  expect(await screen.findByText('ana-fixes')).toBeTruthy();

  // Remove asks first; keeping it changes nothing.
  await fireEvent.press(screen.getByRole('button', { name: fill(w.removeWhat, 'Venmo') }));
  const sheet = await screen.findByTestId('payment-confirm');
  expect(sheet.props.accessibilityViewIsModal).toBe(true);
  expect(screen.getByText(fill(w.confirmTitle, 'Venmo'))).toBeTruthy();
  expect(screen.getByText(w.confirmText)).toBeTruthy();
  await fireEvent.press(screen.getByRole('button', { name: w.keep }));
  await waitFor(() => expect(screen.queryByTestId('payment-confirm')).toBeNull());
  expect(client.removePaymentHandle).not.toHaveBeenCalled();

  await fireEvent.press(screen.getByRole('button', { name: fill(w.removeWhat, 'Venmo') }));
  await fireEvent.press(await screen.findByTestId('payment-confirm-remove'));
  expect(await screen.findByText(fill(w.removedOne, 'Venmo'))).toBeTruthy();
  expect(client.removePaymentHandle).toHaveBeenLastCalledWith('venmo');

  // The last one: the confirmation says showing them turns off on every yup.
  await fireEvent.press(screen.getByRole('button', { name: fill(w.removeWhat, 'Zelle') }));
  expect(await screen.findByText(w.confirmLast)).toBeTruthy();
  await fireEvent.press(screen.getByTestId('payment-confirm-remove'));
  expect(await screen.findByText(fill(w.removedLast, 'Zelle'))).toBeTruthy();
  expect(screen.getByText(w.empty)).toBeTruthy();
});

test('with all four added there is nothing to add, and a refusal is said', async () => {
  const client = handlesClient(DANA);
  client.setPaymentHandle.mockRejectedValueOnce(new ApiFailure('TOO_MANY_REQUESTS'));
  await render(wrap(<PaymentOptionsScreen client={client} />));
  expect(await screen.findByText(w.allAdded)).toBeTruthy();
  expect(screen.queryByRole('button', { name: w.add })).toBeNull();
  await fireEvent.press(screen.getByRole('button', { name: fill(w.editWhat, 'PayPal') }));
  await fireEvent.changeText(screen.getByLabelText(w.paypalLabel), 'AnaPays');
  await fireEvent.press(screen.getByRole('button', { name: w.saveOne }));
  expect(await screen.findByText(wording.errors.TOO_MANY_REQUESTS)).toBeTruthy();
  expect(screen.getByLabelText(w.paypalLabel).props.value).toBe('AnaPays');
});

test('a payee turns showing them on and off with a switch that starts off', async () => {
  const view: ExchangeView = { ...exchange(null), you: 'A' };
  const client = {
    paymentHandles: async () => DANA,
    setPaymentOptions: jest.fn(async (_id: string, on: boolean) => ({ on })),
  };
  const reload = jest.fn(async () => null);
  await render(wrap(<ShowPaymentOptions exchange={view} otherName="Ben Ortiz" reload={reload} client={client} />));
  const toggle = await screen.findByTestId('show-payment-options');
  expect(toggle.props.accessibilityLabel).toBe(w.showLabel);
  expect(toggle.props.accessibilityState).toMatchObject({ checked: false });
  expect(toggle.props.accessibilityHint).toBe(w.showHint.replace('{name}', 'Ben Ortiz'));
  // The options themselves are changed on their own screen.
  await fireEvent.press(screen.getByRole('link', { name: w.manage }));
  expect(mockPush).toHaveBeenCalledWith('/account/payments');
  await fireEvent(toggle, 'valueChange', true);
  expect(await screen.findByText(w.shownNow.replace('{name}', 'Ben Ortiz'))).toBeTruthy();
  expect(client.setPaymentOptions).toHaveBeenCalledWith(view.id, true);
  expect(reload).toHaveBeenCalled();

  client.setPaymentOptions.mockRejectedValueOnce(new ApiFailure('TOO_MANY_REQUESTS'));
  await fireEvent(screen.getByTestId('show-payment-options'), 'valueChange', false);
  expect(await screen.findByText(wording.errors.TOO_MANY_REQUESTS)).toBeTruthy();
});

test('nothing is offered to a party who pays and receives no money', async () => {
  const client = { paymentHandles: async () => DANA, setPaymentOptions: jest.fn() };
  await render(
    wrap(<ShowPaymentOptions exchange={exchange(null)} otherName="Ana Ruiz" reload={async () => null} client={client} />),
  );
  await waitFor(() => expect(screen.queryByText(w.showHeading)).toBeNull());
});

test('an option the payee changed recently is warned about beside its button', async () => {
  const view = exchange(DANA, false, { ...NO_CHANGES, zelle: '2026-10-07T15:30:00Z' });
  await render(
    wrap(
      <PaySheet
        exchange={view}
        contribution={view.in_force_revision!.terms.contributions[0]}
        otherName="Ana Ruiz"
        onClose={() => {}}
        onPaid={() => {}}
        client={{ getExchange: async () => view }}
        open={() => {}}
      />,
    ),
  );
  const warning = await screen.findByText(
    /^Ana Ruiz changed this Zelle email or phone number on .*2026.*\. Check with Ana Ruiz another way before you pay\.$/,
  );
  expect(warning).toBeTruthy();
  expect(screen.queryAllByText(/changed this Venmo username/)).toHaveLength(0);
  // Unchanged options say nothing more.
  expect(screen.getByRole('button', { name: w.open.venmo }).props.accessibilityHint).toBe(
    'Venmo @dana-fixes. Opens with $100.50 and the note filled in. Check both before you send.',
  );
});
