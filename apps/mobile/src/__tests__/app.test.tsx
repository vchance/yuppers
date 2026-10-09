import { SMS_CODE_CONSENT_VERSION, wordingFor } from '@yuppers/shared';
import { router } from 'expo-router';
import { act, fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';
import { Platform } from 'react-native';

import { redirectSystemPath } from '../app/+native-intent';
import { forgetInvitation, heldInvitation } from '../lib/invitation';
import {
  DRAFT,
  EXCHANGE,
  fakeService,
  INVITATION,
  REPAIR,
  signInOnScreen,
  TOKEN,
  ana,
  type FakeService,
} from './fake-service';

/*
 * The whole app, routes and all, run as iOS and as Android builds would run
 * it: the platform's own files are the ones loaded, never the browser
 * stand-ins. There is no simulator on the machines that run this, so it is
 * the nearest thing to opening the app, short of a device. Native modules
 * are replaced by the test preset's mocks.
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

// The Android date dialog is a native module with nothing behind it here; the
// iOS picker is an ordinary native view and is rendered as it is.
jest.mock('@react-native-community/datetimepicker', () => {
  const { Platform: platform } = jest.requireActual('react-native');
  if (platform.OS === 'ios') return jest.requireActual('@react-native-community/datetimepicker');
  return {
    __esModule: true,
    default: () => null,
    DateTimePickerAndroid: { open: jest.fn(), dismiss: jest.fn() },
  };
});

// The installed build's version, which the test preset's mock leaves out. A
// build that cannot read one names none (`lib/client-identity.ts`).
jest.mock('expo-application', () => ({ nativeApplicationVersion: '1.2.0' }));

// The device keeps the exchanges' own time zone unless a test says otherwise.
const mockDeviceZone = { current: 'America/Chicago' };
jest.mock('../lib/time-zone', () => ({ deviceTimezone: () => mockDeviceZone.current }));

jest.mock('expo-crypto', () => {
  let next = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, '0')}` };
});

// The client takes hold of `fetch` when it is made, so the stand-in for the
// service is in place before any of the app is loaded.
let service: FakeService = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = wordingFor('en');

/**
 * Starts the app at an address, signed in or not, with a service that can
 * send codes to phone numbers unless `phone` is false.
 */
async function open(
  initialUrl: string,
  { signedIn, phone = true }: { signedIn: boolean; phone?: boolean },
) {
  mockKeychain.clear();
  service = fakeService();
  service.phone = phone;
  if (signedIn) {
    mockKeychain.set('yuppers.session', TOKEN);
    service.account = ana;
  }
  // The router's own answers hang off what `renderRouter` returns, so that is
  // handed back as it is, not unwrapped.
  const app = renderRouter('src/app', { initialUrl });
  await app;
  return { app };
}

afterEach(() => {
  forgetInvitation();
  mockDeviceZone.current = 'America/Chicago';
});

test('this is a device build, not the browser harness', () => {
  expect(['ios', 'android']).toContain(Platform.OS);
});

test('signing in keeps the token in secure storage and nowhere else, then asks for the profile', async () => {
  await open('/', { signedIn: false });

  // Nobody is signed in: the first screen is the way to sign in. The app
  // has asked the service nothing but how old a build may be and what it
  // can send codes to.
  await screen.findByText(w.signIn.intro);
  expect(new Set(service.sent.map((request) => request.path))).toEqual(new Set(['/v1/meta']));
  expect(service.sent[0].body).toBeNull();

  await fireEvent.changeText(
    screen.getByLabelText(w.signIn.identifierLabel),
    ' ana@example.test ',
  );
  // No box for an email address.
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.press(screen.getByText(w.signIn.sendCode));
  await screen.findByLabelText(w.signIn.codeLabel);
  expect(service.sent.at(-1)).toMatchObject({
    path: '/v1/auth/codes',
    body: { identifier: 'ana@example.test' },
  });

  await fireEvent.changeText(screen.getByLabelText(w.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.signIn.submit }));

  // A new account is asked for a name and its age before anything else.
  await screen.findByText(w.profile.firstIntro);
  expect(service.sent.find((request) => request.path === '/v1/auth/sessions')?.body).toEqual({
    identifier: 'ana@example.test',
    code: '123456',
    delivery: 'TOKEN',
    language: 'en',
  });
  expect(mockKeychain.get('yuppers.session')).toBe(TOKEN);

  // Neither is assumed: leaving them out is refused here.
  await fireEvent.press(screen.getByText(w.profile.continue));
  await screen.findByText(w.profile.nameRequired);
  await screen.findByText(w.profile.adultRequired);

  await fireEvent.changeText(screen.getByLabelText(w.profile.nameLabel), 'Ana Ruiz');
  await fireEvent(screen.getByLabelText(w.profile.adultLabel), 'valueChange', true);
  await fireEvent.press(screen.getByText(w.profile.continue));

  await screen.findByText(w.home.title);
  expect(service.sent.at(-2)).toMatchObject({
    method: 'PATCH',
    path: '/v1/me',
    authorization: `Bearer ${TOKEN}`,
    body: { display_name: 'Ana Ruiz', adult_confirmed: true },
  });
});

test('where the service has no text messages, signing in asks for an email address only', async () => {
  await open('/', { signedIn: false, phone: false });
  await screen.findByText(w.signIn.introEmail);
  expect(screen.queryByText(w.signIn.intro)).toBeNull();
  expect(screen.queryByLabelText(w.signIn.identifierLabel)).toBeNull();
  const email = screen.getByLabelText(w.signIn.emailLabel);
  // The keyboard and the system's suggestions are for an email address.
  expect(email.props.inputMode).toBe('email');
  expect(email.props.textContentType).toBe('emailAddress');
  expect(email.props.autoComplete).toBe('email');

  // A phone number typed anyway is stopped here, before anything is sent,
  // and with no box: no code would go by text.
  await fireEvent.changeText(email, '+1 555 123 4567');
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.press(screen.getByText(w.signIn.sendCode));
  await screen.findByText(w.signIn.emailOnly);
  expect(screen.queryByText(w.errors.SERVICE_UNAVAILABLE)).toBeNull();
  expect(service.sent.some((request) => request.path === '/v1/auth/codes')).toBe(false);

  // An email address goes through, and going back offers another one.
  await fireEvent.changeText(email, 'ana@example.test');
  expect(screen.queryByText(w.signIn.emailOnly)).toBeNull();
  await fireEvent.press(screen.getByText(w.signIn.sendCode));
  await screen.findByLabelText(w.signIn.codeLabel);
  expect(service.sent.at(-1)).toMatchObject({
    path: '/v1/auth/codes',
    body: { identifier: 'ana@example.test' },
  });
  expect(screen.getByText(w.signIn.changeEmail)).toBeTruthy();
});

test('where the service sends text messages, a phone number is asked for and sent', async () => {
  await open('/', { signedIn: false });
  await screen.findByText(w.signIn.intro);
  const identifier = screen.getByLabelText(w.signIn.identifierLabel);
  // Only +1 is texted: no country code is asked for.
  expect(identifier.props.accessibilityHint).toContain(w.signIn.identifierHintUs);
  expect(identifier.props.accessibilityHint).not.toContain('country code');
  expect(identifier.props.textContentType).toBe('username');
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.changeText(identifier, '+15552345678');

  // The box, unticked, named by the words the terms quote, its two
  // addresses links; "Send code" waits for it and says why.
  const box = screen.getByRole('checkbox');
  expect(box.props.accessibilityLabel).toBe(w.smsCode.signIn);
  expect(w.smsCode.signIn).toBe(
    'Text me a one-time sign-in code from yuppers.app at this number. One message per request. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.',
  );
  expect(box.props.accessibilityState).toMatchObject({ checked: false });
  expect(
    screen
      .getAllByRole('link')
      .map((link) => link.props.children)
      .filter((text) => /^https:/.test(String(text))),
  ).toEqual(['https://yuppers.app/terms', 'https://yuppers.app/privacy']);
  const send = () => screen.getByRole('button', { name: w.signIn.sendCode });
  expect(send().props.accessibilityState).toMatchObject({ disabled: true });
  expect(send().props.accessibilityHint).toBe(w.smsCode.tickToSend);
  screen.getByText(w.smsCode.tickToSend);
  // The short line it replaces is gone.
  expect(screen.queryByText('Message and data rates may apply. Reply STOP to opt out.')).toBeNull();
  await fireEvent.press(send());
  expect(service.sent.some((request) => request.path === '/v1/auth/codes')).toBe(false);

  // Another number unticks it, even typed back.
  await fireEvent.press(box);
  expect(screen.getByRole('checkbox').props.accessibilityState).toMatchObject({ checked: true });
  await fireEvent.changeText(identifier, '+15552345679');
  expect(screen.getByRole('checkbox').props.accessibilityState).toMatchObject({ checked: false });
  // Typed without +1, the American way: the same number, still unticked.
  await fireEvent.changeText(identifier, '555-234-5678');
  expect(screen.getByRole('checkbox').props.accessibilityState).toMatchObject({ checked: false });

  await fireEvent.press(screen.getByRole('checkbox'));
  // Leaving the field writes the number the American way; the box stays
  // ticked, for it is the same number.
  await fireEvent(screen.getByLabelText(w.signIn.identifierLabel), 'blur');
  expect(screen.getByLabelText(w.signIn.identifierLabel).props.value).toBe('(555) 234-5678');
  expect(screen.getByRole('checkbox').props.accessibilityState).toMatchObject({ checked: true });
  expect(send().props.accessibilityState).toMatchObject({ disabled: false });
  expect(send().props.accessibilityHint).toBeUndefined();
  expect(screen.queryByText(w.smsCode.tickToSend)).toBeNull();
  await fireEvent.press(send());
  await screen.findByLabelText(w.signIn.codeLabel);
  expect(service.sent.at(-1)).toMatchObject({
    path: '/v1/auth/codes',
    body: {
      identifier: '+15552345678',
      sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'en' },
    },
  });
  screen.getByText(w.signIn.codeSent.replace('{identifier}', '(555) 234-5678'));

  // Back to the number: unticked again.
  await fireEvent.press(screen.getByText(w.signIn.changeIdentifier));
  expect((await screen.findByRole('checkbox')).props.accessibilityState).toMatchObject({
    checked: false,
  });
});

test('a session from an earlier launch opens straight onto the exchanges', async () => {
  await open('/', { signedIn: true });
  await screen.findByText(w.home.title);
  await screen.findByText('With Ben Ortiz');
  // Besides asking how old a build may be, which needs no session.
  const asked = service.sent.filter((request) => request.path !== '/v1/meta');
  expect(asked.map((request) => request.path)).toEqual(['/v1/me', '/v1/exchanges']);
  for (const request of asked) expect(request.authorization).toBe(`Bearer ${TOKEN}`);
  expect(service.sent.some((request) => request.path === '/v1/meta')).toBe(true);
  // Every request names the client and its build.
  for (const request of service.sent) {
    expect(request.clientVersion).toMatch(/^(ios|android)\/1\.2\.0$/);
  }
});

test('a token the service no longer honors is dropped, and the app asks to sign in', async () => {
  await open('/', { signedIn: true });
  await screen.findByText(w.home.title);

  mockKeychain.clear();
  service = fakeService();
  mockKeychain.set('yuppers.session', 'an-old-token');
  await renderRouter('src/app', { initialUrl: '/' });
  await screen.findByText(w.signIn.intro);
  await waitFor(() => expect(mockKeychain.has('yuppers.session')).toBe(false));
});

test('an action on an exchange names its version and carries its own key', async () => {
  await open(`/exchanges/${EXCHANGE}`, { signedIn: true });

  // The agreement in force, in full, with dates and money in the reader's language.
  await screen.findByText('Yup with Ben Ortiz');
  screen.getByText('Due October 30, 2026');
  screen.getByText('Amount: $450.00');
  screen.getByText('2 required items are still to be confirmed.');

  // Marking delivered opens a panel first; nothing is sent until it is confirmed.
  await fireEvent.press(screen.getByText(w.exchange.moves.CLAIM));
  expect(service.sent.some((request) => request.path.endsWith('/commands'))).toBe(false);
  await fireEvent.changeText(screen.getByLabelText(w.exchange.noteLabel), 'Done this morning');
  await fireEvent.press(screen.getAllByText(w.exchange.moves.CLAIM).at(-1)!);

  await screen.findByText(w.contributionStatus.CLAIMED);
  const command = service.sent.find((request) => request.path.endsWith('/commands'));
  expect(command).toMatchObject({
    method: 'POST',
    path: `/v1/exchanges/${EXCHANGE}/commands`,
    authorization: `Bearer ${TOKEN}`,
    body: {
      expected_version: 7,
      command: {
        type: 'CONTRIBUTION',
        contribution: REPAIR,
        action: 'CLAIM',
        note: 'Done this morning',
      },
    },
  });
  expect(command?.idempotencyKey).toMatch(/^[0-9a-f-]{36}$/);
  await screen.findByText(w.exchange.updated);
});

test('when the exchange changed underneath, it is reloaded and the person is told', async () => {
  await open(`/exchanges/${EXCHANGE}`, { signedIn: true });
  await screen.findByText('Yup with Ben Ortiz');

  service.conflictNext = true;
  await fireEvent.press(screen.getByText(w.exchange.moves.CLAIM));
  await fireEvent.press(screen.getAllByText(w.exchange.moves.CLAIM).at(-1)!);

  // The refusal says what happened, and the screen shows what the exchange is now.
  await screen.findByText(w.errors.VERSION_CONFLICT);
  await screen.findByText('Ben Ortiz proposed ending this agreement.');
  // The repair is spoken of as a delivery and the payment as money.
  expect(screen.getAllByText(w.contributionStatus.PENDING)).toHaveLength(1);
  expect(screen.getAllByText(w.moneyStatus.PENDING)).toHaveLength(1);
  // The history is read again with it, since the exchange is a newer version.
  await waitFor(() => {
    const after = service.sent.slice(service.sent.findIndex((r) => r.path.endsWith('/commands')));
    expect(after.map((request) => `${request.method} ${request.path}`)).toEqual([
      `POST /v1/exchanges/${EXCHANGE}/commands`,
      `GET /v1/exchanges/${EXCHANGE}`,
      `GET /v1/exchanges/${EXCHANGE}/history`,
    ]);
  });
});

test('a due date names the exchange’s time zone for a device that keeps another', async () => {
  mockDeviceZone.current = 'Europe/Madrid';
  await open(`/exchanges/${EXCHANGE}`, { signedIn: true });
  await screen.findByText('Yup with Ben Ortiz');
  screen.getByText(w.terms.dueOnDateInZone.replace('{date}', 'October 30, 2026').replace('{zone}', 'Chicago'));
  expect(screen.queryByText('Due October 30, 2026')).toBeNull();
});

test('a draft opens in the composer, with the platform’s own date control', async () => {
  await open(`/exchanges/${DRAFT}`, { signedIn: true });
  await screen.findByText(w.composer.titleFirst);
  expect(screen.getByLabelText(w.composer.otherName).props.defaultValue).toBe('Ben Ortiz');
  screen.getByText(w.composer.dateLabel);
  await fireEvent.changeText(
    await screen.findByLabelText(w.invitationLink.forLabel),
    'ben@example.test',
  );

  // Reviewing shows the complete terms and the consent step; nothing is sent yet.
  await fireEvent.press(screen.getByText(w.composer.review));
  await screen.findByText(w.composer.signIntro);
  screen.getByText('Due October 30, 2026');
  screen.getByText(w.consent.pendingReview);
  expect(screen.getByTestId('consent-sign').props.accessibilityState).toMatchObject({
    disabled: true,
  });
  expect(service.sent.some((request) => request.path.endsWith('/revisions'))).toBe(false);
});

test('an invitation address gives up its token: it is sent in a body and kept out of the route', async () => {
  const { app } = await open(`/en/i#${INVITATION}`, { signedIn: false });

  // Signed out, the screen asks to sign in, and nothing is asked about the link.
  await screen.findByText(w.invitation.signInToRead);
  screen.getByRole('header', { name: w.invitation.signedOutTitle });
  expect(app.getPathnameWithParams()).toBe('/invitation');
  expect(heldInvitation()).toBe(INVITATION);
  expect(service.sent.some((request) => request.path.startsWith('/v1/invitations/'))).toBe(false);
  expect(screen.queryByText(w.invitation.notBinding)).toBeNull();

  // Signed in, the proposal is read, with the session.
  await signInOnScreen(w);
  await screen.findByText(w.invitation.notBinding);
  screen.getByRole('header', { name: w.invitation.title });
  const preview = service.sent.find((request) => request.path === '/v1/invitations/preview');
  expect(preview).toMatchObject({
    method: 'POST',
    authorization: `Bearer ${TOKEN}`,
    body: { token: INVITATION },
  });
  expect(app.getPathnameWithParams()).toBe('/invitation');
  for (const request of service.sent) expect(request.path).not.toContain(INVITATION);
  // Nothing is claimed by reading.
  expect(service.sent.some((request) => request.path === '/v1/invitations/claim')).toBe(false);
});

test('signed in as someone new, responding asks for the profile and then claims', async () => {
  const { app } = await open(`/en/i#${INVITATION}`, { signedIn: false });
  await screen.findByText(w.invitation.signInToRead);
  await signInOnScreen(w);
  await screen.findByText(w.invitation.notBinding);

  await fireEvent.press(screen.getByRole('button', { name: w.invitation.respondNew }));
  await screen.findByText(w.profile.firstIntro);
  // Signing in is not asked for again.
  expect(screen.queryByLabelText(w.signIn.identifierLabel)).toBeNull();
  await fireEvent.changeText(screen.getByLabelText(w.profile.nameLabel), 'Ben Ortiz');
  await fireEvent(screen.getByLabelText(w.profile.adultLabel), 'valueChange', true);
  await fireEvent.press(screen.getByText(w.profile.continue));

  await waitFor(() => expect(app.getPathnameWithParams()).toBe(`/exchanges/${EXCHANGE}`));
  expect(
    service.sent.filter((request) => request.path === '/v1/invitations/claim').map((r) => r.body),
  ).toEqual([{ token: INVITATION }]);
  expect(service.sent.filter((request) => request.path === '/v1/auth/codes')).toHaveLength(1);
});

test('a second invitation link arriving while one is open shows the new proposal', async () => {
  await open(`/en/i#${INVITATION}`, { signedIn: true });
  await screen.findByText(w.invitation.notBinding);

  // The system hands the app another link, as it does when one is tapped
  // with the app already open on an invitation.
  const other = 'b4'.repeat(32);
  await act(async () => {
    router.navigate(redirectSystemPath({ path: `yuppers://es/i#${other}`, initial: false }));
  });
  await waitFor(() =>
    expect(service.sent.at(-1)).toMatchObject({
      path: '/v1/invitations/preview',
      body: { token: other },
    }),
  );
  await screen.findByText(w.invitation.notBinding);
  expect(heldInvitation()).toBe(other);
  for (const request of service.sent) expect(request.path).not.toContain(other);
});

/** The claims the app sent, plain or only asking whether the place is already this account's. */
const claims = () =>
  service.sent
    .filter((request) => request.path === '/v1/invitations/claim')
    .map((request) => request.body);

test('a used link takes the person who used it back to the exchange, and claims nothing', async () => {
  mockKeychain.clear();
  mockKeychain.set('yuppers.session', TOKEN);
  service = fakeService();
  service.account = ana;
  service.invitation = 'yours';
  const app = renderRouter('src/app', { initialUrl: `/en/i#${INVITATION}` });
  await app;

  await waitFor(() => expect(app.getPathnameWithParams()).toBe(`/exchanges/${EXCHANGE}`));
  // Only asked whether the place was already Ana's; nothing that could take one.
  expect(claims()).toEqual([{ token: INVITATION, only_if_yours: true }]);
  expect(heldInvitation()).toBeNull();
});

test('a link someone else used is refused, and opening it never claims', async () => {
  mockKeychain.clear();
  mockKeychain.set('yuppers.session', TOKEN);
  service = fakeService();
  service.account = ana;
  service.invitation = 'spent';
  const app = renderRouter('src/app', { initialUrl: `/en/i#${INVITATION}` });
  await app;

  await screen.findByText(w.errors.INVITATION_UNAVAILABLE);
  expect(app.getPathnameWithParams()).toBe('/invitation');
  expect(claims()).toEqual([{ token: INVITATION, only_if_yours: true }]);
  // Nothing on the screen claims it either: there is no way to respond.
  expect(screen.queryByText(w.invitation.respondNew)).toBeNull();
  expect(
    screen.queryByText(w.invitation.respondAs.replace('{name}', ana.display_name)),
  ).toBeNull();
  expect(screen.queryByText(w.invitation.notBinding)).toBeNull();
  expect(heldInvitation()).toBeNull();
});

test('a live link is claimed only when the person taps to respond', async () => {
  const { app } = await open(`/en/i#${INVITATION}`, { signedIn: true });
  await screen.findByText(w.invitation.notBinding);
  expect(claims()).toEqual([]);

  await fireEvent.press(
    screen.getByRole('button', { name: w.invitation.respondAs.replace('{name}', ana.display_name) }),
  );
  await waitFor(() => expect(app.getPathnameWithParams()).toBe(`/exchanges/${EXCHANGE}`));
  expect(claims()).toEqual([{ token: INVITATION }]);
});

/** A notification email's link, as the system hands it to the app as a universal or app link. */
const emailed = (path = '') =>
  redirectSystemPath({ path: `https://yuppers.example/exchanges/${EXCHANGE}${path}`, initial: true });

test('a notification email’s link to an exchange opens its screen', async () => {
  const { app } = await open(emailed(), { signedIn: true });
  await screen.findByText('Yup with Ben Ortiz');
  expect(app.getPathnameWithParams()).toBe(`/exchanges/${EXCHANGE}`);
});

test('signed out, a notification email’s link asks for sign-in on the way to the exchange', async () => {
  const { app } = await open(emailed(), { signedIn: false });
  await screen.findByText(w.signIn.intro);
  expect(app.getPathnameWithParams()).toBe(`/exchanges/${EXCHANGE}`);
});

test('a notification email’s link to the record opens the record', async () => {
  const { app } = await open(emailed('/record'), { signedIn: true });
  await waitFor(() => expect(app.getPathnameWithParams()).toBe(`/exchanges/${EXCHANGE}/record`));
});

test('the account’s payment options row opens their own screen, and back shows what changed', async () => {
  const { app } = await open('/account', { signedIn: true });
  const row = await screen.findByRole('button', { name: `${w.payments.heading}, ${w.payments.noneAdded}` });
  await fireEvent.press(row);
  await screen.findByText(w.payments.empty);
  expect(app.getPathnameWithParams()).toBe('/account/payments');

  await fireEvent.press(screen.getByRole('button', { name: w.payments.add }));
  await fireEvent.press(await screen.findByRole('button', { name: w.payments.apps.venmo }));
  await fireEvent.changeText(screen.getByLabelText(w.payments.venmoLabel), 'ana-pays');
  await fireEvent.press(screen.getByRole('button', { name: w.payments.saveOne }));
  await screen.findByText('ana-pays');
  expect(service.sent.find((request) => request.path === '/v1/me/payment-handles/venmo')).toMatchObject({
    method: 'PUT',
    body: { value: 'ana-pays' },
  });

  await act(() => router.back());
  await screen.findByRole('button', { name: `${w.payments.heading}, Venmo` });
  expect(app.getPathnameWithParams()).toBe('/account');
});

test('with no link to open, an invitation can be pasted', async () => {
  await open('/invitation', { signedIn: false });
  const pasting = w.mobile.openInvitation;
  await screen.findByText(pasting.intro);

  await fireEvent.changeText(screen.getByLabelText(pasting.label), 'https://example.test/');
  await fireEvent.press(screen.getByText(pasting.open));
  await screen.findByText(pasting.invalid);
  expect(service.sent.filter((request) => request.path !== '/v1/meta')).toEqual([]);

  await fireEvent.changeText(
    screen.getByLabelText(pasting.label),
    `https://app.example/es/i#${INVITATION}`,
  );
  await fireEvent.press(screen.getByText(pasting.open));
  // Signed out, a pasted link leads to signing in first, then the proposal.
  await screen.findByText(w.invitation.signInToRead);
  expect(service.sent.some((request) => request.path.startsWith('/v1/invitations/'))).toBe(false);
  await signInOnScreen(w);
  await screen.findByText(w.invitation.notBinding);
  expect(service.sent.at(-1)).toMatchObject({
    path: '/v1/invitations/preview',
    body: { token: INVITATION },
  });
});
