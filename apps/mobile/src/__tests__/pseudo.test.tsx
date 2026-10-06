import { languages, timeZoneCity } from '@yuppers/shared';
import { formattedWords, pseudoWording, untranslated } from '@yuppers/shared/testing/pseudo';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';
import * as Print from 'expo-print';
import { Platform } from 'react-native';

import { forgetInvitation } from '../lib/invitation';
import { notifications, resetNotifications } from './fake-notifications';
import {
  DRAFT,
  EXCHANGE,
  fakeService,
  INVITATION,
  signInOnScreen,
  TOKEN,
  ana,
  type FakeService,
} from './fake-service';

/*
 * Text written into a component instead of the wording stays in English
 * whatever language the person reads. Here the app runs, as iOS and as
 * Android, in a pseudo-language made from the English wording, with every
 * Latin letter accented (`@yuppers/shared/testing/pseudo`), and the main
 * screens are read for any plain Latin letter that did not come through it:
 * text, labels, hints and placeholders, and the page the record's PDF is
 * printed from.
 *
 * What may still have plain letters, and nothing else:
 *   - what people wrote or chose, as the stand-in service holds it
 *     (`STAND_IN_TEXT` below): names, terms, notes, the currency and zone;
 *   - what the platform formats: month names, day periods, time zone names
 *     and the words joining a date to its time;
 *   - each language's own name, in the language picker, and its tag, as
 *     the record says which language a signature's consent was shown in;
 *   - things that are not words: references, ids, hashes, email addresses,
 *     and an invitation link, which is an address.
 */

jest.mock('@yuppers/shared', () => {
  const actual = jest.requireActual('@yuppers/shared');
  const { pseudoWording: pseudo } = jest.requireActual('@yuppers/shared/testing/pseudo');
  const wording = pseudo();
  return { ...actual, wordingFor: () => wording };
});

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

// The record's PDF is kept as the HTML it is printed from, and not made.
jest.mock('expo-print', () => ({ printToFileAsync: jest.fn() }));
const print = jest.mocked(Print);

let service: FakeService = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = pseudoWording();

/** Text in the stand-in service's data that may show without coming from the wording. */
const STAND_IN_TEXT = [
  'Ana Ruiz',
  'Ben Ortiz',
  'Here is what we talked about on Tuesday.',
  'Repair the back fence.',
  'Repair the back fence',
  'Payment for the repair',
  'USD',
  'America/Chicago',
  // The record's notices, and how a signature was checked, come from the service.
  'about, from the service',
  'signatures, from the service',
  'statements, from the service',
  'content hash, from the service',
  'described by the record',
  // The version of the consent wording a signature was given under.
  'draft-1',
];
const ALLOWED = [
  ...STAND_IN_TEXT,
  ...formattedWords('en', ['America/Chicago']),
  // The exchange's time zone by its city, beside a due date for a device elsewhere.
  timeZoneCity('America/Chicago'),
  ...languages.flatMap((language) => [language.name, language.code]),
];

async function open(
  initialUrl: string,
  signedIn: boolean,
  prepare?: (service: FakeService) => void,
) {
  mockKeychain.clear();
  service = fakeService();
  if (signedIn) {
    mockKeychain.set('yuppers.session', TOKEN);
    service.account = ana;
  }
  prepare?.(service);
  await renderRouter('src/app', { initialUrl });
}

afterEach(() => {
  forgetInvitation();
});

interface Node {
  type: unknown;
  props: Record<string, unknown>;
  children: (Node | string)[];
}

/** Text a person can read or hear: text, and the props that are read out or shown. */
function readable(): string[] {
  const found: string[] = [];
  const walk = (node: Node | string) => {
    if (typeof node === 'string') {
      if (node.trim()) found.push(node);
      return;
    }
    for (const name of ['accessibilityLabel', 'aria-label', 'accessibilityHint', 'placeholder']) {
      const value = node.props?.[name];
      if (typeof value === 'string' && value) found.push(value);
    }
    for (const child of node.children ?? []) walk(child);
  };
  walk(screen.root as unknown as Node);
  return found;
}

const found = new Set<string>();
/** Notes everything on the screen as it stands that did not come through the wording. */
function check() {
  for (const text of readable()) {
    const words = untranslated(text, ALLOWED);
    if (words.length > 0) found.add(`${JSON.stringify(text.trim())}: ${words.join(' ')}`);
  }
}
beforeEach(() => found.clear());

describe('every word on the main mobile screens comes from the wording', () => {
  test('signing in, and a new account’s profile', async () => {
    await open('/', false);
    await screen.findByText(w.signIn.intro);
    check();
    // A phone number: the box beside it, unticked and then ticked.
    await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), '+12015550123');
    check();
    await fireEvent.press(screen.getByRole('checkbox'));
    check();
    await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), 'ana@example.test');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.sendCode }));
    await screen.findByLabelText(w.signIn.codeLabel);
    check();
    await fireEvent.changeText(screen.getByLabelText(w.signIn.codeLabel), '123456');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.submit }));
    await screen.findByText(w.profile.firstIntro);
    check();
    await fireEvent.press(screen.getByRole('button', { name: w.profile.continue }));
    await screen.findByText(w.profile.nameRequired);
    check();
    expect([...found]).toEqual([]);
  });

  test('an invitation, pasted in and opened from a link, and answered', async () => {
    await open('/', false);
    await screen.findByText(w.mobile.invited.heading);
    await fireEvent.press(screen.getByRole('button', { name: w.mobile.openInvitation.title }));
    await screen.findByText(w.mobile.openInvitation.intro);
    check();
    await open(`/en/i#${INVITATION}`, false);
    await screen.findByText(w.invitation.signInToRead);
    check();
    await signInOnScreen(w);
    await screen.findByText(w.invitation.notBinding);
    check();
    await fireEvent.press(screen.getByRole('button', { name: w.invitation.respondNew }));
    await screen.findByText(w.profile.firstIntro);
    check();
    expect([...found]).toEqual([]);
  });

  test('the list of exchanges and the account', async () => {
    await open('/', true);
    await screen.findByText(w.home.title);
    await screen.findAllByText(/Ben Ortiz/);
    check();
    await open('/account', true);
    await screen.findByText(w.safety.blockedEmpty);
    check();
    await fireEvent.press(screen.getByRole('button', { name: w.deletion.open }));
    check();
    expect([...found]).toEqual([]);
  });

  test('notifications: the offer on the list, the switch, and a phone-only account', async () => {
    notifications.projectId = 'test-project-id';
    try {
      await open('/', true, (fake) => {
        fake.push = true;
      });
      await screen.findByText(w.mobile.notifications.askHeading);
      check();
      notifications.permission = { granted: false, canAskAgain: false };
      await open('/account', true, (fake) => {
        fake.push = true;
        fake.account = { ...ana, email: null, phone: '+15555550123' };
      });
      await screen.findByText(w.mobile.notifications.blocked);
      await screen.findByText(w.mobile.notifications.phoneOnly);
      check();
    } finally {
      resetNotifications();
    }
    await open('/account', true, (fake) => {
      fake.account = { ...ana, email: null, phone: '+15555550123' };
    });
    await screen.findByText(w.mobile.notifications.phoneOnlyNoPush);
    check();
    expect([...found]).toEqual([]);
  });

  test('the composer, writing and then signing', async () => {
    await open(`/exchanges/${DRAFT}`, true);
    await screen.findByText(w.composer.titleFirst);
    check();
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await screen.findByText(w.composer.signIntro);
    check();
    expect([...found]).toEqual([]);
  });

  test('who the invitation is for, then the link, its QR code and waiting for a code', async () => {
    await open(`/exchanges/${DRAFT}`, true);
    const field = await screen.findByLabelText(w.invitationLink.forLabel);
    await fireEvent.changeText(field, 'carla@');
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await screen.findByText(w.invitationLink.forInvalid);
    check();
    await fireEvent.changeText(
      screen.getByLabelText(w.invitationLink.forLabel),
      'carla@example.test',
    );
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await screen.findByText(w.composer.signIntro);
    check();
    await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
    await fireEvent.press(screen.getByTestId('consent-sign'));
    await screen.findByText(w.invitationLink.intro);
    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.shareQr }));
    screen.getByTestId('qr-code');
    check();
    expect([...found]).toEqual([]);
  });

  test('an agreement in force, its panels, the guide and the record', async () => {
    await open(`/exchanges/${EXCHANGE}`, true);
    await screen.findByText(w.exchange.title.replace('{name}', 'Ben Ortiz'));
    // The wallet button, which comes once the service has said it has passes.
    await screen.findByText(Platform.OS === 'ios' ? w.wallet.addToApple : w.wallet.addToGoogle);
    check();
    await fireEvent.press(screen.getByRole('button', { name: w.exchange.moves.CLAIM }));
    check();
    await fireEvent.press(screen.getByRole('button', { name: w.common.cancel }));
    await fireEvent.press(screen.getByRole('button', { name: w.trouble.open }));
    check();
    await fireEvent.press(
      screen.getByRole('button', {
        name: w.trouble.situations.THEY_HAVENT.replace('{name}', 'Ben Ortiz'),
      }),
    );
    check();
    await open(`/exchanges/${EXCHANGE}/record`, true);
    await screen.findByText(w.record.summary.heading);
    check();

    let printed = '';
    print.printToFileAsync.mockImplementation(async ({ html } = {}) => {
      printed = html ?? '';
      throw new Error('not printed in a test');
    });
    await fireEvent.press(screen.getByRole('button', { name: w.mobile.record.savePdf }));
    await waitFor(() => expect(printed).toContain(w.record.summary.heading));
    const page = printed
      .replace(/<style[\s\S]*?<\/style>/g, ' ')
      .replace(/<[^>]*>/g, '\n')
      .replace(/&(amp|lt|gt|quot|#39);/g, ' ');
    for (const text of page.split('\n')) {
      const words = untranslated(text, ALLOWED);
      if (words.length > 0) found.add(`PDF ${JSON.stringify(text.trim())}: ${words.join(' ')}`);
    }
    expect([...found]).toEqual([]);
  });
});
