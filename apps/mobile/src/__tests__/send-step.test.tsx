import { wordingFor } from '@yuppers/shared';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';
import * as Clipboard from 'expo-clipboard';
import { Linking, Share } from 'react-native';

import { forgetInvitation } from '../lib/invitation';
import {
  DRAFT,
  EXCHANGE,
  SENT_INVITATION,
  TOKEN,
  ana,
  fakeService,
  waitingExchange,
} from './fake-service';

/*
 * Sending the invitation is a step of its own after signing (DESIGN.md §8):
 * Yuppers never sends it, so the person who signed is asked to, plainly,
 * with the way that reaches the person named first, and can only say "I’ll
 * send it later" to go past it. Whatever they open is recorded, and the
 * exchange's screen and the list say where the link stands until someone joins.
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
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn(async () => true) }));

let service = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = wordingFor('en');
const LINK = `http://localhost:5173/en/i#${SENT_INVITATION}`;
const MESSAGE = encodeURIComponent(`I’ve sent you a yup to review: ${LINK}`);

async function open(initialUrl: string, before?: (service: typeof fakeService extends () => infer T ? T : never) => void) {
  mockKeychain.clear();
  service = fakeService();
  mockKeychain.set('yuppers.session', TOKEN);
  service.account = ana;
  before?.(service);
  await renderRouter('src/app', { initialUrl });
}

/** Writes the draft's first proposal for `invitee` and signs it, landing on the step. */
async function signAndSend(invitee: string | null) {
  await open(`/exchanges/${DRAFT}`);
  const field = await screen.findByLabelText(w.invitationLink.forLabel);
  if (invitee === null) {
    await fireEvent.press(screen.getByText(w.invitationLink.forAnyone));
    await screen.findByText(w.invitationLink.forAnyoneText);
  } else {
    await fireEvent.changeText(field, invitee);
  }
  await fireEvent.press(screen.getByText(w.composer.review));
  await screen.findByText(w.composer.signIntro);
  await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
  await fireEvent.press(screen.getByTestId('consent-sign'));
  await screen.findByText('Send it to Ben Ortiz');
}

function sharedCalls(): number {
  return service.sent.filter(
    (request) => request.path === `/v1/exchanges/${DRAFT}/invitation/shared`,
  ).length;
}

let opened: jest.SpyInstance;
let shared: jest.SpyInstance;
beforeEach(() => {
  opened = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
  shared = jest.spyOn(Share, 'share').mockResolvedValue({ action: 'sharedAction' });
});

afterEach(() => {
  forgetInvitation();
  jest.restoreAllMocks();
});

describe('the step that sends the link', () => {
  test('named by phone: a text message to the number first, the rule said plainly, and the way on once opened', async () => {
    await signAndSend('2025550142');
    screen.getByText(
      'Yuppers doesn’t send it for you. Ben Ortiz gets nothing until you share this link.',
    );
    screen.getByText('Only someone who signs in with (202) 555-0142 will be able to use the link.');
    screen.getByText(LINK);
    // Not sending is a choice of its own, said to leave a reminder.
    screen.getByRole('link', { name: w.invitationLink.later });
    screen.getByText('You can send it from the yup’s page. It reminds you until Ben Ortiz joins.');
    expect(screen.queryByRole('button', { name: w.invitationLink.sendDone })).toBeNull();
    expect(screen.queryByRole('button', { name: w.invitationLink.share })).toBeNull();
    expect(sharedCalls()).toBe(0);

    // Texting it opens Messages with her number and the message, and is recorded.
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.sendText }));
    expect(opened).toHaveBeenCalledWith(`sms:+12025550142?body=${MESSAGE}`);
    await waitFor(() => expect(sharedCalls()).toBe(1));
    await screen.findByText(
      'Once it’s sent, Ben Ortiz can read the yup after signing in. You’ll see on the yup’s page when they join.',
    );
    expect(screen.queryByRole('link', { name: w.invitationLink.later })).toBeNull();

    // WhatsApp beside it, and copying.
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.sendWhatsApp }));
    expect(opened).toHaveBeenLastCalledWith(`https://wa.me/?text=${MESSAGE}`);
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.copy }));
    expect(Clipboard.setStringAsync).toHaveBeenCalledWith(LINK);
    await screen.findByText(w.invitationLink.copied);
    await waitFor(() => expect(sharedCalls()).toBe(3));

    // Done: the exchange, whose card says the link was shared.
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.sendDone }));
    await screen.findByText('Yup with Ben Ortiz');
    screen.getByText('Ben Ortiz hasn’t joined yet');
    screen.getByText(/^You shared the link on /);
    screen.getByText(w.invitationLink.unclaimed);
    expect(screen.queryByText(/You haven’t sent them the link/)).toBeNull();
  });

  test('named by email: an email to the address first, and copying beside it', async () => {
    await signAndSend('carla@example.test');
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.sendEmail }));
    expect(opened).toHaveBeenCalledWith(
      `mailto:carla@example.test?subject=${encodeURIComponent(w.linkPreview.title)}&body=${MESSAGE}`,
    );
    expect(screen.queryByRole('button', { name: w.invitationLink.sendText })).toBeNull();
    expect(screen.queryByRole('button', { name: w.invitationLink.sendWhatsApp })).toBeNull();
    screen.getByRole('button', { name: w.invitationLink.copy });
    await waitFor(() => expect(sharedCalls()).toBe(1));
  });

  test('for anyone: the share sheet first, with messages to nobody in particular beside it', async () => {
    await signAndSend(null);
    screen.getByText(w.invitationLink.forAnyoneSummary);
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.share }));
    expect(shared).toHaveBeenCalledTimes(1);
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.shareSms }));
    expect(opened).toHaveBeenLastCalledWith(`sms:?body=${MESSAGE}`);
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.shareEmail }));
    expect(opened).toHaveBeenLastCalledWith(
      `mailto:?subject=${encodeURIComponent(w.linkPreview.title)}&body=${MESSAGE}`,
    );
    screen.getByRole('button', { name: w.invitationLink.shareWhatsApp });
    await waitFor(() => expect(sharedCalls()).toBe(3));
  });

  test('“I’ll send it later” goes to the exchange, which reminds until the link is sent', async () => {
    await signAndSend('2025550142');
    await fireEvent.press(screen.getByRole('link', { name: w.invitationLink.later }));
    await screen.findByText('Yup with Ben Ortiz');
    expect(sharedCalls()).toBe(0);

    // The reminder: headed with who has not joined, saying nothing reaches
    // them, and still holding the link and the ways to send it.
    screen.getByText('Ben Ortiz hasn’t joined yet');
    screen.getByText(
      'You haven’t sent them the link. Yuppers doesn’t send it for you: Ben Ortiz gets nothing until you do.',
    );
    expect(screen.queryByText(w.invitationLink.unclaimed)).toBeNull();
    screen.getByText(LINK);
    screen.getByText(w.invitationLink.reissueIntro);

    // Sending it from here is recorded too, and the reminder goes.
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.sendText }));
    expect(opened).toHaveBeenCalledWith(`sms:+12025550142?body=${MESSAGE}`);
    await waitFor(() => expect(sharedCalls()).toBe(1));
    await screen.findByText(/^You shared the link on /);
    expect(screen.queryByText(/You haven’t sent them the link/)).toBeNull();
  });
});

describe('the reminder on the exchange’s screen, on coming back', () => {
  test('a link never sent: the reminder, and a new link as the way to send it', async () => {
    await open(`/exchanges/${EXCHANGE}`, (fake) => {
      fake.exchange = waitingExchange(null);
    });
    await screen.findByText('Ben Ortiz hasn’t joined yet');
    screen.getByText(/You haven’t sent them the link/);
    // The link cannot be shown again: a new one is the way to send it.
    screen.getByText(w.invitationLink.sendAgain);
    expect(screen.queryByText(w.invitationLink.reissueIntro)).toBeNull();
    expect(screen.queryByRole('button', { name: w.invitationLink.sendText })).toBeNull();
    screen.getByRole('button', { name: w.invitationLink.reissue });
  });

  test('a link sent long ago: the reminder says when, and to send it again', async () => {
    await open(`/exchanges/${EXCHANGE}`, (fake) => {
      fake.exchange = waitingExchange('2026-09-01T09:00:00Z');
    });
    await screen.findByText('Ben Ortiz hasn’t joined yet');
    screen.getByText(
      /^You shared the link on September 1, 2026 at .*, and nobody has joined through it\. If Ben Ortiz hasn’t seen it, send it again, or create a new link\.$/,
    );
    screen.getByText(w.invitationLink.sendAgain);
  });

  test('a link sent a moment ago: no reminder, only where things stand', async () => {
    await open(`/exchanges/${EXCHANGE}`, (fake) => {
      fake.exchange = waitingExchange(new Date().toISOString());
    });
    await screen.findByText('Ben Ortiz hasn’t joined yet');
    screen.getByText(/^You shared the link on /);
    screen.getByText(w.invitationLink.unclaimed);
    screen.getByText(w.invitationLink.reissueIntro);
    expect(screen.queryByText(/You haven’t sent them the link/)).toBeNull();
    expect(screen.queryByText(w.invitationLink.sendAgain)).toBeNull();
  });
});

describe('the list', () => {
  test('marks a yup whose link was never sent, read with the rest of the card', async () => {
    await open('/', (fake) => {
      fake.exchange = waitingExchange(null);
    });
    await screen.findByText(w.home.notSent);
    const card = screen.getByRole('button', { name: /Not sent yet/ });
    expect(card.props.accessibilityLabel).toMatch(
      /^Waiting to be signed\. Not sent yet\. Reference PVVS-5Q2K\. Updated October 2, 2026 at .*\. With Ben Ortiz$/,
    );
  });

  test('once sent, the chip says who it waits for', async () => {
    await open('/', (fake) => {
      fake.exchange = waitingExchange('2026-10-02T06:35:00Z');
    });
    await screen.findByText('Waiting for Ben Ortiz');
    expect(screen.queryByText(w.home.notSent)).toBeNull();
  });

  test('nothing is said once someone has joined', async () => {
    await open('/');
    await screen.findByText(w.states.ACTIVE);
    expect(screen.queryByText(w.home.notSent)).toBeNull();
    expect(screen.queryByText(/^Waiting for /)).toBeNull();
  });
});
