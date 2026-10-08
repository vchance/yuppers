import { wordingFor } from '@yuppers/shared';
import { act, fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';

import { forgetInvitation } from '../lib/invitation';
import { CODE_SENDER, DRAFT, SENT_INVITATION, TOKEN, ana, fakeService } from './fake-service';
import { readQr } from './read-qr';

/*
 * The first use of the app, end to end on the screens, as iOS and as Android
 * builds run it: asking for who the invitation is for beside their name,
 * the link once it is sent with its QR code, and waiting for a sign-in code.
 */

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
jest.mock('expo-application', () => ({ nativeApplicationVersion: '1.2.0' }));
jest.mock('../lib/time-zone', () => ({ deviceTimezone: () => 'America/Chicago' }));
jest.mock('expo-crypto', () => {
  let next = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, '0')}` };
});

let service = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = wordingFor('en');

async function open(
  initialUrl: string,
  {
    signedIn,
    phone = true,
    codeSender = CODE_SENDER,
  }: { signedIn: boolean; phone?: boolean; codeSender?: string | null },
) {
  mockKeychain.clear();
  service = fakeService();
  service.phone = phone;
  service.codeSender = codeSender;
  if (signedIn) {
    mockKeychain.set('yuppers.session', TOKEN);
    service.account = ana;
  }
  await renderRouter('src/app', { initialUrl });
}

afterEach(() => {
  forgetInvitation();
  jest.useRealTimers();
});

describe('who the invitation is for', () => {
  test('is asked beside their name, checked before signing, named on the signing step, and sent', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    const field = await screen.findByLabelText(w.invitationLink.forLabel);

    // Something plainly wrong is said, and the signing step is not reached.
    await fireEvent.changeText(field, 'carla@');
    await fireEvent.press(screen.getByText(w.composer.review));
    await screen.findByText(w.invitationLink.forInvalid);
    expect(screen.queryByText(w.composer.signIntro)).toBeNull();
    screen.getByText('1 thing needs fixing before you can sign.');

    // Put right, the signing step names who it is for, and asks nothing more.
    await fireEvent.changeText(
      screen.getByLabelText(w.invitationLink.forLabel),
      'carla@example.test',
    );
    expect(screen.queryByText(w.invitationLink.forInvalid)).toBeNull();
    await fireEvent.press(screen.getByText(w.composer.review));
    await screen.findByText(w.composer.signIntro);
    screen.getByText('Only someone who signs in with carla@example.test will be able to use the link.');
    expect(screen.queryByLabelText(w.invitationLink.forLabel)).toBeNull();

    await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
    await fireEvent.press(screen.getByTestId('consent-sign'));
    await screen.findByText(w.invitationLink.intro);
    const sent = service.sent.find(
      (request) => request.path === `/v1/exchanges/${DRAFT}/revisions`,
    );
    expect(sent?.body).toMatchObject({ invitation: { bound_to: 'carla@example.test' } });

    // The link, with the way to show it as a QR code.
    const link = screen.getByText(new RegExp(`/en/i#${SENT_INVITATION}$`));
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.shareQr }));
    expect(readQr(screen.getByTestId('qr-code') as never)).toBe(link.props.children);
    screen.getByRole('button', { name: w.invitationLink.share });
    screen.getByRole('button', { name: w.invitationLink.copy });
  });

  test('left empty, it is asked for: an empty field never makes a link for anyone', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    const field = await screen.findByLabelText(w.invitationLink.forLabel);
    // Why naming them helps, that we do not contact them, and that it must match.
    screen.getByText(w.invitationLink.forIntro);
    screen.getByText(w.invitationLink.forNoContact);
    expect(field.props.accessibilityHint).toContain(w.invitationLink.forHint);

    await fireEvent.press(screen.getByText(w.composer.review));
    await screen.findByText(w.invitationLink.forMissing);
    expect(screen.queryByText(w.composer.signIntro)).toBeNull();
    expect(service.sent.some((request) => request.path.endsWith('/revisions'))).toBe(false);
  });

  test('a link for anyone is a deliberate choice that says what it costs, and names nobody', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    await fireEvent.changeText(await screen.findByLabelText(w.invitationLink.forLabel), 'carla@');
    await fireEvent.press(screen.getByText(w.invitationLink.forAnyone));
    await screen.findByText(w.invitationLink.forAnyoneText);
    expect(screen.queryByLabelText(w.invitationLink.forLabel)).toBeNull();

    // Changing one's mind back keeps what was typed.
    await fireEvent.press(screen.getByText(w.invitationLink.forNamed));
    expect(screen.getByLabelText(w.invitationLink.forLabel).props.value).toBe('carla@');
    await fireEvent.press(screen.getByText(w.invitationLink.forAnyone));

    // What was typed is neither checked nor sent.
    await fireEvent.press(screen.getByText(w.composer.review));
    await screen.findByText(w.composer.signIntro);
    screen.getByText(w.invitationLink.forAnyoneSummary);
    await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
    await fireEvent.press(screen.getByTestId('consent-sign'));
    await screen.findByText(w.invitationLink.intro);
    const sent = service.sent.find(
      (request) => request.path === `/v1/exchanges/${DRAFT}/revisions`,
    );
    expect(sent?.body).toEqual(expect.objectContaining({ invitation: { for_anyone: true } }));
  });

  test('takes a US number typed without +1, written the American way, and sends it in E.164', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    const field = () => screen.getByLabelText(w.invitationLink.forLabel);
    await screen.findByLabelText(w.invitationLink.forLabel);

    await fireEvent.changeText(field(), '2025550142');
    await fireEvent(field(), 'blur');
    expect(field().props.value).toBe('(202) 555-0142');
    await fireEvent.press(screen.getByText(w.composer.review));
    await screen.findByText(w.composer.signIntro);
    screen.getByText('Only someone who signs in with (202) 555-0142 will be able to use the link.');

    await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
    await fireEvent.press(screen.getByTestId('consent-sign'));
    await waitFor(() =>
      expect(
        service.sent.find((request) => request.path === `/v1/exchanges/${DRAFT}/revisions`)?.body,
      ).toEqual(expect.objectContaining({ invitation: { bound_to: '+12025550142' } })),
    );
  });

  test('is an email address only where codes go by email only', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true, phone: false });
    const field = await screen.findByLabelText(w.invitationLink.forLabelEmail);
    expect(field.props.keyboardType).toBe('email-address');
    expect(screen.queryByLabelText(w.invitationLink.forLabel)).toBeNull();

    await fireEvent.changeText(field, '+1 202 555 0142');
    await fireEvent.press(screen.getByText(w.composer.review));
    await screen.findByText(w.invitationLink.forEmailOnly);
    expect(screen.queryByText(w.composer.signIntro)).toBeNull();
  });
});

describe('waiting for a sign-in code', () => {
  async function requestCode(identifier = 'ana@example.test') {
    await screen.findByText(w.signIn.intro);
    await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), identifier);
    // A number gets its code by text only with the box beside it ticked.
    if (!identifier.includes('@')) await fireEvent.press(screen.getByRole('checkbox'));
    await fireEvent.press(screen.getByText(w.signIn.sendCode));
    await screen.findByLabelText(w.signIn.codeLabel);
  }

  test('says to look in the spam folder too, for an email from the address the service names', async () => {
    await open('/', { signedIn: false });
    await requestCode();
    screen.getByText(`Check your inbox, and your spam folder, for an email from ${CODE_SENDER}.`);
  });

  test('without an address from the service, the folders alone; for a phone number, nothing', async () => {
    await open('/', { signedIn: false, codeSender: null });
    await requestCode();
    screen.getByText(w.signIn.checkInboxAnySender);

    await fireEvent.press(screen.getByText(w.signIn.changeIdentifier));
    await requestCode('+15552345678');
    expect(screen.queryByText(w.signIn.checkInboxAnySender)).toBeNull();
  });

  test('offers another code only after 30 seconds, then again 30 seconds after that one', async () => {
    jest.useFakeTimers({ advanceTimers: true });
    await open('/', { signedIn: false });
    await requestCode();
    const resend = () => screen.queryByRole('button', { name: w.signIn.resend });

    expect(resend()).toBeNull();
    screen.getByText(w.signIn.resendSoon);
    await act(async () => {
      jest.advanceTimersByTime(29_000);
    });
    expect(resend()).toBeNull();
    await act(async () => {
      jest.advanceTimersByTime(1_000);
    });
    expect(resend()).not.toBeNull();
    expect(screen.queryByText(w.signIn.resendSoon)).toBeNull();

    await fireEvent.press(resend()!);
    await screen.findByText(w.signIn.resent);
    expect(service.sent.filter((request) => request.path === '/v1/auth/codes')).toHaveLength(2);
    expect(resend()).toBeNull();
    await act(async () => {
      jest.advanceTimersByTime(30_000);
    });
    await waitFor(() => expect(resend()).not.toBeNull());
  });

  test('another code over the limit says so, as before', async () => {
    jest.useFakeTimers({ advanceTimers: true });
    await open('/', { signedIn: false });
    await requestCode();
    await act(async () => {
      jest.advanceTimersByTime(30_000);
    });
    service.refuseCodes = 'TOO_MANY_REQUESTS';
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.resend }));
    await screen.findByText(w.errors.TOO_MANY_REQUESTS);
    expect(screen.queryByText(w.signIn.resent)).toBeNull();
  });
});
