import type { ExchangeView } from '@yuppers/api-client';
import {
  ApiFailure,
  createI18n,
  wordingFor,
  type WalletApi,
  type WalletPlatform,
} from '@yuppers/shared';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { Linking, Platform } from 'react-native';

import { I18nContext } from '../../lib/context';
import { WalletButton } from '../WalletButton';

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));

const wording = wordingFor('en');
const i18n = createI18n('en', wording, () => {});
const ID = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70';
const APPLE_LINK = 'https://app.test/v1/wallet/apple/pass?token=t';
const GOOGLE_LINK = 'https://pay.google.com/gp/v/save/jwt';

function exchange(state: ExchangeView['state']): ExchangeView {
  return {
    id: ID,
    version: 4,
    state,
    you: 'B',
    counterparty: 'CONFIRMED',
    currency: 'USD',
    timezone: 'America/Chicago',
    display_code: 'ABCD-1234',
    contributions: [],
  };
}

/** A stand-in for the service's Wallet calls. */
function service(platforms: WalletPlatform[], refuse = false): WalletApi {
  return {
    meta: async () => ({
      service: 'yuppers-backend',
      version: '0.0.0',
      commit: 'unknown',
      minimum_client_versions: {},
      push_notifications: false,
      wallet_platforms: platforms,
      sign_in_channels: ['email'],
      sms_country_codes: [],
      sms_updates: false,
    }),
    appleWalletLink: async () => {
      if (refuse) throw new ApiFailure('TOO_MANY_REQUESTS');
      return { url: APPLE_LINK, expires_at: null };
    },
    googleWalletLink: async () => ({ url: GOOGLE_LINK }),
  };
}

function show(props: Partial<Parameters<typeof WalletButton>[0]>) {
  return render(
    <I18nContext.Provider value={i18n}>
      <WalletButton exchange={exchange('ACTIVE')} {...props} />
    </I18nContext.Provider>,
  );
}

test('the device’s own wallet is offered, and the link is handed to the system', async () => {
  const open = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
  await show({ client: service(['APPLE', 'GOOGLE']) });
  const ios = Platform.OS === 'ios';
  const button = await screen.findByRole('button', {
    name: ios ? wording.wallet.addToApple : wording.wallet.addToGoogle,
  });
  // On iOS the pass opens in Safari, which the button says.
  expect(button.props.accessibilityHint).toBe(ios ? wording.help.inBrowser : undefined);
  expect(
    screen.getByText(wording.wallet.intro.replace('{productName}', wording.productName)),
  ).toBeTruthy();
  await fireEvent.press(button);
  await waitFor(() => expect(open).toHaveBeenCalledWith(ios ? APPLE_LINK : GOOGLE_LINK));
  open.mockRestore();
});

test('no button where the service has no passes for the device’s wallet, or nothing is in force', async () => {
  const other: WalletPlatform = Platform.OS === 'ios' ? 'GOOGLE' : 'APPLE';
  const meta = jest.fn(service([other]).meta);
  await show({ client: { ...service([other]), meta } });
  await waitFor(() => expect(meta).toHaveBeenCalled());
  expect(screen.queryByRole('button')).toBeNull();

  for (const state of ['NEGOTIATING', 'CLOSED'] as const) {
    await render(
      <I18nContext.Provider value={i18n}>
        <WalletButton exchange={exchange(state)} client={service(['APPLE', 'GOOGLE'])} />
      </I18nContext.Provider>,
    );
    expect(screen.queryByRole('button')).toBeNull();
  }

  // A device with neither wallet asks nothing.
  const asked = jest.fn(service(['APPLE', 'GOOGLE']).meta);
  await show({ client: { ...service(['APPLE', 'GOOGLE']), meta: asked }, device: null });
  expect(screen.queryByRole('button')).toBeNull();
  expect(asked).not.toHaveBeenCalled();
});

test('a refusal is said, and the button stays', async () => {
  const open = jest.fn(async () => {});
  await show({ client: service(['APPLE'], true), device: 'APPLE', open });
  await fireEvent.press(await screen.findByRole('button', { name: wording.wallet.addToApple }));
  expect(await screen.findByText(wording.errors.TOO_MANY_REQUESTS)).toBeTruthy();
  expect(open).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: wording.wallet.addToApple })).toBeTruthy();
});
