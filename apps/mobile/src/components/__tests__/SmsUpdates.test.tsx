import type { Account, ExchangeView } from '@yuppers/api-client';
import {
  ApiFailure,
  createI18n,
  SMS_CONSENT_VERSION,
  wordingFor,
  type SetSmsUpdates,
  type SmsUpdates as Standing,
  type SmsUpdatesApi,
} from '@yuppers/shared';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { AccessibilityInfo, Linking } from 'react-native';

import { I18nContext, SessionContext, type Session } from '../../lib/context';
import { SmsUpdates } from '../SmsUpdates';

/*
 * "Text updates" on an agreement (DESIGN.md §12): adding a US number with a
 * code by text, the box beside the consent wording as the terms quote it,
 * what Save says, a number that replied STOP, and where nothing is shown.
 */

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));

const ID = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70';
const PHONE = '+15552345678';

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

const account: Account = {
  id: 'a0000000-0000-4000-8000-000000000001',
  display_name: 'Ana Ruiz',
  adult_confirmed: true,
  language: 'en',
  email: 'ana@example.test',
};

interface Stand extends SmsUpdatesApi {
  phone: string | null;
  on: boolean;
  optedOut: boolean;
  texting: boolean;
  codes: string[];
  saved: SetSmsUpdates[];
}

/** A stand-in for the service's calls. */
function stand(prepare: Partial<Stand> = {}): Stand {
  const service: Stand = {
    phone: null,
    on: false,
    optedOut: false,
    texting: true,
    codes: [],
    saved: [],
    ...prepare,
    meta: async () => ({
      service: 'yuppers-backend',
      version: '0.0.0',
      commit: 'unknown',
      minimum_client_versions: {},
      push_notifications: false,
      wallet_platforms: [],
      sign_in_channels: ['email', 'phone'],
      sms_country_codes: ['+1'],
      sms_updates: service.texting,
    }),
    smsUpdates: async () => standing(service, 'ACTIVE'),
    setSmsUpdates: async (_id: string, body: SetSmsUpdates) => {
      service.saved.push(body);
      if (body.on && service.optedOut) throw new ApiFailure('PHONE_OPTED_OUT');
      service.on = body.on;
      return standing(service, 'ACTIVE');
    },
    requestCode: async (identifier: string) => {
      service.codes.push(identifier);
    },
    addIdentifier: async (identifier: string, code: string) => {
      if (code !== '123456') throw new ApiFailure('INVALID_CODE');
      service.phone = identifier;
      return { ...account, phone: identifier };
    },
  };
  return service;
}

function standing(service: Stand, state: string): Standing {
  return {
    on: service.on,
    available: service.texting && ['NEGOTIATING', 'ACTIVE'].includes(state),
    phone: service.phone,
    opted_out: service.optedOut,
    consent_version: SMS_CONSENT_VERSION,
  };
}

async function show(client: SmsUpdatesApi, language: 'en' | 'es' = 'en', state: ExchangeView['state'] = 'ACTIVE') {
  const wording = wordingFor(language);
  const session = { setAccount: jest.fn() } as unknown as Session;
  await render(
    <I18nContext.Provider value={createI18n(language, wording, () => {})}>
      <SessionContext.Provider value={session}>
        <SmsUpdates exchange={exchange(state)} client={client} />
      </SessionContext.Provider>
    </I18nContext.Provider>,
  );
  return { wording, session };
}

let announced: jest.SpyInstance;
beforeEach(() => {
  announced = jest.spyOn(AccessibilityInfo, 'announceForAccessibility');
});
afterEach(() => jest.restoreAllMocks());

test('a party with no number adds one with a code, then ticks the box and saves', async () => {
  const service = stand();
  const { wording, session } = await show(service);
  const w = wording.smsUpdates;
  expect(await screen.findByRole('header', { name: w.heading })).toBeTruthy();
  expect(screen.getByText(wording.privacy.sms)).toBeTruthy();

  // Not a US number: said, and nothing sent.
  await fireEvent.changeText(screen.getByLabelText(w.phoneLabel), '+44 7700 900123');
  await fireEvent.press(screen.getByRole('button', { name: w.sendCode }));
  expect(await screen.findByText(w.phoneInvalid)).toBeTruthy();
  expect(service.codes).toEqual([]);

  await fireEvent.changeText(screen.getByLabelText(w.phoneLabel), '(555) 234-5678');
  await fireEvent.press(screen.getByRole('button', { name: w.sendCode }));
  expect(await screen.findByText('We texted a code to +1 •••-•••-5678.')).toBeTruthy();
  expect(service.codes).toEqual([PHONE]);

  await fireEvent.changeText(screen.getByLabelText(w.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.addPhone }));
  const box = await screen.findByRole('checkbox');
  expect(session.setAccount).toHaveBeenCalledWith({ ...account, phone: PHONE });
  expect(screen.getByText('+1 •••-•••-5678 is now on your account.')).toBeTruthy();

  // The box is named by the consent wording, word for word, which is shown
  // beside it with its two addresses as links to the browser.
  expect(box.props.accessibilityLabel).toBe(w.consent);
  expect(box.props.accessibilityState).toMatchObject({ checked: false });
  const links = screen.getAllByRole('link').filter((link) => /^https:/.test(String(link.props.children)));
  expect(links.map((link) => link.props.children)).toEqual([
    'https://yuppers.app/terms',
    'https://yuppers.app/privacy',
  ]);
  const open = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
  await fireEvent.press(links[0]);
  expect(open).toHaveBeenCalledWith('https://yuppers.app/terms');

  // Ticked and saved: on, with the consent's version and language.
  await fireEvent.press(box);
  expect(screen.getByRole('checkbox').props.accessibilityState).toMatchObject({ checked: true });
  await fireEvent.press(screen.getByRole('button', { name: w.save }));
  const confirmation =
    'Text updates are on for this agreement. You’ll get one text per status change at +1 •••-•••-5678. Reply STOP to opt out.';
  expect(await screen.findByText(confirmation)).toBeTruthy();
  expect(service.saved).toEqual([
    { on: true, consent: { version: SMS_CONSENT_VERSION, language: 'en' } },
  ]);
  expect(announced).toHaveBeenCalledWith(confirmation);

  // Unticked and saved: off.
  await fireEvent.press(screen.getByRole('checkbox'));
  await fireEvent.press(screen.getByRole('button', { name: w.save }));
  expect(await screen.findByText(w.off)).toBeTruthy();
  expect(service.saved.at(-1)).toEqual({ on: false });
  expect(service.on).toBe(false);
});

test('the page on how people opt in opens in the browser, in the language shown', async () => {
  const open = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
  const { wording } = await show(stand({ phone: PHONE }), 'es');
  await fireEvent.press(await screen.findByRole('link', { name: wording.smsUpdates.howItWorks }));
  expect(open).toHaveBeenCalledWith(expect.stringMatching(/\/es\/sms-opt-in$/));
  expect((await screen.findByRole('checkbox')).props.accessibilityLabel).toBe(
    'Recibir actualizaciones por mensaje de texto de yuppers.app sobre este acuerdo, un mensaje por cada cambio de estado. La frecuencia de los mensajes varía; no hay un máximo fijo. Pueden aplicarse tarifas por mensajes y datos. Responde HELP para obtener ayuda o STOP para cancelar. Términos: https://yuppers.app/terms. Política de privacidad: https://yuppers.app/privacy.',
  );
});

test('a number that replied STOP is told how to get texts again, and offered no box', async () => {
  const { wording } = await show(stand({ phone: PHONE, optedOut: true }));
  expect(
    await screen.findByText(wording.smsUpdates.optedOut.replace('{phone}', '+1 •••-•••-5678')),
  ).toBeTruthy();
  expect(screen.queryByRole('checkbox')).toBeNull();
});

test('nothing is shown where texts are not sent, or on a closed agreement', async () => {
  const off = stand({ texting: false });
  const meta = jest.fn(off.meta);
  await show({ ...off, meta });
  await waitFor(() => expect(meta).toHaveBeenCalled());
  expect(screen.queryByRole('header')).toBeNull();

  const closed = stand({ phone: PHONE });
  const smsUpdates = jest.fn(async () => standing(closed, 'CLOSED'));
  await show({ ...closed, smsUpdates }, 'en', 'CLOSED');
  await waitFor(() => expect(smsUpdates).toHaveBeenCalled());
  expect(screen.queryByRole('header')).toBeNull();
});
