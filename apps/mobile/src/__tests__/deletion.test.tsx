import { deletedNotice, formatMessage, SMS_CODE_CONSENT_VERSION, wordingFor } from '@yuppers/shared';
import * as SecureStore from 'expo-secure-store';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';

import { EXCHANGE, fakeService, TOKEN, ana, type FakeService } from './fake-service';

/*
 * Deleting the account, in the whole app, as iOS and as Android builds would
 * run it: the explanation, the code, the last confirmation, and what is left
 * on the device afterwards. The stand-in for the service is the shared one,
 * with the three deletion calls answered here.
 */

// The device's secure storage, in memory.
const mockKeychain = new Map<string, string>();
jest.mock('expo-secure-store', () => ({
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: 'when-unlocked-this-device-only',
  getItemAsync: jest.fn(async (key: string) => mockKeychain.get(key) ?? null),
  setItemAsync: jest.fn(async (key: string, value: string) => {
    mockKeychain.set(key, value);
  }),
  deleteItemAsync: jest.fn(async (key: string) => {
    mockKeychain.delete(key);
  }),
}));

jest.mock('@react-native-community/datetimepicker', () => {
  const { Platform: platform } = jest.requireActual('react-native');
  if (platform.OS === 'ios') return jest.requireActual('@react-native-community/datetimepicker');
  return {
    __esModule: true,
    default: () => null,
    DateTimePickerAndroid: { open: jest.fn(), dismiss: jest.fn() },
  };
});

jest.mock('expo-crypto', () => {
  let next = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, '0')}` };
});

const CODE = '654321';

let service: FakeService = fakeService();
/** What deleting would do, as the service counts it. */
let preview = { drafts: 0, open_proposals: 0, agreements_in_force: 0 };

/** The deletion calls, answered the way the service answers them. */
function deletion(method: string, path: string, authorized: boolean, body: unknown) {
  if (!authorized || !service.account) return [401, { code: 'UNAUTHENTICATED' }] as const;
  const call = `${method} ${path}`;
  if (call === 'GET /v1/me/deletion') return [200, preview] as const;
  if (call === 'POST /v1/me/deletion/codes') {
    // Like the service: a code by text only with the box beside the number ticked.
    const { channel, sms_consent } = body as { channel: string; sms_consent?: unknown };
    if (channel === 'PHONE' && !sms_consent) {
      return [422, { code: 'SMS_CONSENT_REQUIRED' }] as const;
    }
    return [204, null] as const;
  }
  if (call === 'POST /v1/me/deletion') {
    if ((body as { code?: string }).code !== CODE) return [401, { code: 'INVALID_CODE' }] as const;
    // Every session ends with the account.
    service.account = null;
    return [204, null] as const;
  }
  return [404, { code: 'NOT_FOUND' }] as const;
}

globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const request = input instanceof Request ? input : new Request(input, init);
  const path = new URL(request.url).pathname;
  if (!path.startsWith('/v1/me/deletion')) return service.fetch(request);

  const text = await request.text();
  const body = text ? JSON.parse(text) : null;
  const authorization = request.headers.get('Authorization');
  service.sent.push({
    method: request.method,
    path,
    authorization,
    idempotencyKey: null,
    clientVersion: request.headers.get('X-Client-Version'),
    body,
  });
  const [status, answer] = deletion(request.method, path, authorization === `Bearer ${TOKEN}`, body);
  return new Response(answer === null ? null : JSON.stringify(answer), {
    status,
    headers: answer === null ? {} : { 'Content-Type': 'application/json' },
  });
}) as typeof fetch;

const w = wordingFor('en');
const d = w.deletion;
const fmt = (message: string, values: Record<string, string | number>) =>
  formatMessage(message, values, 'en');

/** Starts the app at an address, signed in as Ana, against a service set up by `prepare`. */
async function open(initialUrl: string, prepare: (service: FakeService) => void = () => {}) {
  await openApp(initialUrl, prepare);
}

/** The same, handing back the router's own answers, such as where it is. */
async function openApp(initialUrl: string, prepare: (service: FakeService) => void = () => {}) {
  mockKeychain.clear();
  deletedNotice.dismiss();
  service = fakeService();
  mockKeychain.set('yuppers.session', TOKEN);
  service.account = ana;
  preview = { drafts: 0, open_proposals: 0, agreements_in_force: 0 };
  prepare(service);
  const app = renderRouter('src/app', { initialUrl });
  await app;
  return { app };
}

const deletionCalls = () =>
  service.sent
    .filter((request) => request.path.startsWith('/v1/me/deletion'))
    .map((request) => `${request.method} ${request.path}`);

test('deleting the account says what it does, takes a code and a last confirmation, and leaves no session on the device', async () => {
  await open('/account', () => {
    preview = { drafts: 1, open_proposals: 0, agreements_in_force: 2 };
  });

  // The way in is on the account screen, and nothing is asked of the
  // service until it is opened.
  await fireEvent.press(await screen.findByText(d.open));
  expect(deletionCalls()).toEqual(['GET /v1/me/deletion']);

  // What goes, what happens to the exchanges the account is in, and what
  // stays: in particular the agreements the other party signed.
  await screen.findByText(fmt(d.drafts, { count: 1 }));
  screen.getByText(fmt(d.agreementsInForce, { count: 2 }));
  screen.getByText(d.agreementsStand);
  screen.getByText(d.otherPartyTold);
  screen.getByText(d.deletedAccount);
  screen.getByText(d.keptAgreements);
  expect(screen.queryByText(d.nothingOpen)).toBeNull();
  expect(screen.queryByText(fmt(d.openProposals, { count: 0 }))).toBeNull();

  // With one identifier there is nothing to choose: the code goes to it.
  screen.getByText(fmt(d.codeIntro, { identifier: 'ana@example.test' }));
  await fireEvent.press(screen.getByText(d.sendCode));
  await screen.findByText(fmt(d.codeSent, { identifier: 'ana@example.test' }));
  expect(service.sent.at(-1)).toMatchObject({
    method: 'POST',
    path: '/v1/me/deletion/codes',
    authorization: `Bearer ${TOKEN}`,
    body: { channel: 'EMAIL' },
  });

  // No code, no going on.
  await fireEvent.press(screen.getByText(d.continue));
  await screen.findByText(d.codeRequired);

  // A code leads to the last question, and still nothing has been deleted.
  await fireEvent.changeText(screen.getByLabelText(w.signIn.codeLabel), ` ${CODE} `);
  await fireEvent.press(screen.getByText(d.continue));
  await screen.findByText(d.confirmBody);
  expect(deletionCalls()).toEqual(['GET /v1/me/deletion', 'POST /v1/me/deletion/codes']);
  expect(mockKeychain.get('yuppers.session')).toBe(TOKEN);

  await fireEvent.press(screen.getByText(d.confirm));

  // The app is back at the way to sign in, and says what happened.
  await screen.findByText(w.signIn.intro);
  await screen.findByText(d.deleted);
  // Besides asking what the service can send sign-in codes to.
  expect(service.sent.filter((request) => request.path !== '/v1/meta').at(-1)).toMatchObject({
    method: 'POST',
    path: '/v1/me/deletion',
    authorization: `Bearer ${TOKEN}`,
    body: { channel: 'EMAIL', code: CODE },
  });
  // Nothing of the session is left in the device's secure storage, and
  // nothing more was sent in its name.
  await waitFor(() => expect(mockKeychain.has('yuppers.session')).toBe(false));
  expect(screen.queryByText(w.nav.signOut)).toBeNull();

  await fireEvent.press(screen.getByText(d.dismiss));
  expect(screen.queryByText(d.deleted)).toBeNull();
  screen.getByText(w.signIn.intro);
  // The dead token was never tried again: the deletion was the last request
  // besides asking what the service can send sign-in codes to.
  expect(service.sent.filter((request) => request.path !== '/v1/meta').at(-1)?.path).toBe(
    '/v1/me/deletion',
  );
});

test('the notice that the account was deleted stays when the account screen was opened on its own', async () => {
  // Opened by a direct link: there is no list beneath the account screen,
  // so going back to the first screen mounts it afresh.
  const { app } = await openApp('/account');
  await fireEvent.press(await screen.findByText(d.open));
  await fireEvent.press(await screen.findByText(d.sendCode));
  await fireEvent.changeText(await screen.findByLabelText(w.signIn.codeLabel), CODE);
  await fireEvent.press(screen.getByText(d.continue));
  await screen.findByText(d.confirmBody);

  // Forgetting the session on the device takes a moment, as secure storage
  // does on a phone. The first screen must not appear while the account is
  // still known, or it would take the notice for something already read.
  let release = () => {};
  const forgetting = new Promise<void>((resolve) => {
    release = resolve;
  });
  jest.mocked(SecureStore.deleteItemAsync).mockImplementationOnce(async (key: string) => {
    await forgetting;
    mockKeychain.delete(key);
  });
  const pressed = fireEvent.press(screen.getByText(d.confirm));
  // Timers are fake in these tests: time passes only when it is told to.
  await jest.advanceTimersByTimeAsync(50);
  release();
  await pressed;

  await screen.findByText(w.signIn.intro);
  await screen.findByText(d.deleted);
  expect(app.getPathnameWithParams()).toBe('/');
  expect(mockKeychain.has('yuppers.session')).toBe(false);
});

test('a code that is wrong goes back to asking for it, and the account and its session stay', async () => {
  // An account with both identifiers chooses where the code goes.
  await open('/account', (service) => {
    service.account = { ...ana, phone: '+12025550142' };
  });
  await fireEvent.press(await screen.findByText(d.open));
  await screen.findByText(d.nothingOpen);
  expect(screen.queryByText(d.agreementsStand)).toBeNull();
  screen.getByText(d.codeChoice);
  // By email, the first offered, there is no box.
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.press(screen.getByRole('radio', { name: '+12025550142' }));

  // To the phone: a box, unticked, named by the words for deleting, and
  // the button waits for it, saying why.
  const box = () => screen.getByRole('checkbox');
  expect(box().props.accessibilityLabel).toBe(w.smsCode.deleteAccount);
  expect(w.smsCode.deleteAccount).toBe(
    'Text me a one-time code to confirm deleting your account from yuppers.app at this number. One message per request. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.',
  );
  expect(box().props.accessibilityState).toMatchObject({ checked: false });
  const send = () => screen.getByRole('button', { name: d.sendCode });
  expect(send().props.accessibilityState).toMatchObject({ disabled: true });
  expect(send().props.accessibilityHint).toBe(w.smsCode.tickToSend);
  await fireEvent.press(send());
  expect(deletionCalls()).toEqual(['GET /v1/me/deletion']);

  // Ticked, then by email and back: unticked, never remembered.
  await fireEvent.press(box());
  expect(send().props.accessibilityState).toMatchObject({ disabled: false });
  await fireEvent.press(screen.getByRole('radio', { name: 'ana@example.test' }));
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.press(screen.getByRole('radio', { name: '+12025550142' }));
  expect(box().props.accessibilityState).toMatchObject({ checked: false });

  await fireEvent.press(box());
  await fireEvent.press(send());
  await screen.findByText(fmt(d.codeSent, { identifier: '+12025550142' }));
  expect(service.sent.at(-1)).toMatchObject({
    path: '/v1/me/deletion/codes',
    body: { channel: 'PHONE', sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'en' } },
  });

  await fireEvent.changeText(screen.getByLabelText(w.signIn.codeLabel), '000000');
  await fireEvent.press(screen.getByText(d.continue));
  await fireEvent.press(await screen.findByText(d.confirm));

  // Refused: back at the code, with the reason, and still signed in.
  await screen.findByText(w.errors.INVALID_CODE);
  screen.getByLabelText(w.signIn.codeLabel);
  expect(screen.queryByText(d.confirmBody)).toBeNull();
  expect(service.sent.at(-1)).toMatchObject({
    path: '/v1/me/deletion',
    body: { channel: 'PHONE', code: '000000' },
  });
  expect(mockKeychain.get('yuppers.session')).toBe(TOKEN);
  expect(deletedNotice.shown()).toBe(false);

  // A new code can be asked for, and cancelling leaves everything as it was.
  await fireEvent.press(screen.getByText(w.signIn.resend));
  await screen.findByText(w.signIn.resent);
  await fireEvent.press(screen.getByText(w.common.cancel));
  await screen.findByText(d.open);
  screen.getByText(w.nav.signOut);
  expect(deletionCalls().filter((call) => call === 'POST /v1/me/deletion')).toHaveLength(1);
});

test('an account that never finished setting up can still be deleted', async () => {
  await open('/', (service) => {
    service.account = { ...ana, display_name: '', adult_confirmed: false };
  });
  // No name and no confirmation of age: the account screen is out of reach,
  // so the way to delete the account is here, under what is being asked for.
  await screen.findByText(w.profile.firstIntro);
  await fireEvent.press(screen.getByText(d.open));
  await screen.findByText(d.nothingOpen);
  await fireEvent.press(screen.getByText(d.sendCode));
  await fireEvent.changeText(await screen.findByLabelText(w.signIn.codeLabel), CODE);
  await fireEvent.press(screen.getByText(d.continue));
  await fireEvent.press(await screen.findByText(d.confirm));

  await screen.findByText(d.deleted);
  await screen.findByText(w.signIn.intro);
  await waitFor(() => expect(mockKeychain.has('yuppers.session')).toBe(false));
});

test('someone in an open exchange is told when the other party has deleted their account', async () => {
  await open(`/exchanges/${EXCHANGE}`);
  await screen.findByText('Yup with Ben Ortiz');
  expect(screen.queryByText(fmt(d.otherPartyLeftActive, { name: 'Ben Ortiz' }))).toBeNull();

  await open(`/exchanges/${EXCHANGE}`, (service) => {
    service.exchange = { ...service.exchange, other_party_left: true, close_requested_by: 'B' };
  });
  await screen.findByText(fmt(d.otherPartyLeftActive, { name: 'Ben Ortiz' }));
});
