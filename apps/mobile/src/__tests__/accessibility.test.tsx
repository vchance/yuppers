import { wordingFor } from '@yuppers/shared';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';
import { AccessibilityInfo, Platform, StyleSheet } from 'react-native';

import { forgetInvitation } from '../lib/invitation';
import { notifications, resetNotifications } from './fake-notifications';
import {
  DRAFT,
  EXCHANGE,
  fakeService,
  INVITATION,
  PAYMENT,
  REPAIR,
  signInOnScreen,
  TOKEN,
  ana,
  type FakeService,
} from './fake-service';

/*
 * What a screen reader and a finger meet on the main screens, run as iOS and
 * as Android builds: every control has a role and a name, its state, and a
 * target at least 44 points each way; every text field has a label; each
 * screen is headed; and what changes without the focus moving is said.
 *
 * What this cannot tell: how VoiceOver and TalkBack actually read the
 * screens, how they lay out at the largest text sizes, and whether focus
 * lands where it should on a device. Those need a phone in hand.
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

jest.mock('expo-crypto', () => {
  let next = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, '0')}` };
});

let service: FakeService = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = wordingFor('en');
const MIN_TARGET = 44;

async function open(
  initialUrl: string,
  { signedIn, prepare }: { signedIn: boolean; prepare?: (service: FakeService) => void },
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

let announced: jest.SpyInstance;
let focused: jest.SpyInstance;
beforeEach(() => {
  announced = jest.spyOn(AccessibilityInfo, 'announceForAccessibility');
  focused = jest.spyOn(AccessibilityInfo, 'sendAccessibilityEvent');
});
afterEach(() => {
  forgetInvitation();
  jest.restoreAllMocks();
});

/** A node of the rendered tree, as far as this test looks at it. */
interface Node {
  type: unknown;
  props: Record<string, unknown>;
  children: (Node | string)[];
}

function hostNodes(): Node[] {
  const found: Node[] = [];
  const walk = (node: Node | string) => {
    if (typeof node === 'string') return;
    if (typeof node.type === 'string') found.push(node);
    for (const child of node.children ?? []) walk(child);
  };
  walk(screen.root as unknown as Node);
  return found;
}

/** Whether a node, or something it sits in, is hidden from assistive technology. */
function hidden(node: Node): boolean {
  const { props } = node;
  return (
    props.accessible === false ||
    props['aria-hidden'] === true ||
    props.accessibilityElementsHidden === true ||
    props.importantForAccessibility === 'no' ||
    props.importantForAccessibility === 'no-hide-descendants'
  );
}

/** The text inside a node, as one string. */
function textOf(node: Node | string): string {
  if (typeof node === 'string') return node;
  return (node.children ?? []).map(textOf).join('');
}

const label = (node: Node) =>
  (node.props.accessibilityLabel ?? node.props['aria-label']) as string | undefined;
const role = (node: Node) =>
  (node.props.accessibilityRole ?? node.props.role) as string | undefined;

/**
 * Everything wrong with the screen as it stands, one line each, so a failure
 * says what to fix.
 */
function audit(): string[] {
  const problems: string[] = [];
  const nodes = hostNodes();
  let pressables = 0;
  for (const node of nodes) {
    if (hidden(node)) continue;
    const name = label(node) ?? '(no label)';
    const pressable =
      typeof node.props.onClick === 'function' || typeof node.props.onPress === 'function';
    if (pressable && node.type !== 'Text') {
      pressables += 1;
      if (!role(node)) problems.push(`pressable “${name}” has no role`);
      if (!label(node)) problems.push(`pressable with role ${role(node)} has no label`);
      if (!node.props.accessibilityState && !node.props['aria-disabled'] && role(node) !== 'text') {
        problems.push(`pressable “${name}” says nothing of its state`);
      }
      const style = StyleSheet.flatten(node.props.style as never) as
        { minHeight?: number; minWidth?: number; height?: number } | undefined;
      const tall = Math.max(style?.minHeight ?? 0, style?.height ?? 0);
      if (tall < MIN_TARGET)
        problems.push(`pressable “${name}” is ${tall} tall, under ${MIN_TARGET}`);
    }
    if (node.type === 'TextInput' && !label(node)) problems.push('a text field has no label');
    if (/Switch/.test(String(node.type))) {
      if (!label(node)) problems.push('a switch has no label');
      if (role(node) !== 'switch') problems.push(`switch “${name}” has role ${role(node)}`);
    }
  }
  const headings = nodes.filter((node) => role(node) === 'header' && !hidden(node));
  if (headings.length === 0) problems.push('the screen has no heading');
  // Every heading says how deep it sits, which the browser harness turns
  // into h1 to h4, and one, at level 1, names the screen. A panel's heading
  // sits under the part of the screen it opens in, never at the top.
  for (const heading of headings) {
    const level = heading.props['aria-level'];
    if (typeof level !== 'number' || level < 1 || level > 4) {
      problems.push(`heading “${textOf(heading)}” has no level`);
    }
  }
  const top = headings.filter((heading) => heading.props['aria-level'] === 1);
  if (top.length !== 1) {
    problems.push(
      `the screen has ${top.length} level 1 headings: ${top.map((heading) => `“${textOf(heading)}”`).join(', ')}`,
    );
  }
  if (pressables === 0) problems.push('nothing on the screen can be pressed: is it rendered?');
  return problems;
}

describe('signing in', () => {
  test('the first step', async () => {
    await open('/', { signedIn: false });
    await screen.findByText(w.signIn.intro);
    expect(audit()).toEqual([]);
    const identifier = screen.getByLabelText(w.signIn.identifierLabel);
    // A required field says so, since the platforms have no state for it.
    expect(identifier.props.accessibilityHint).toContain(w.a11y.required);
    expect(identifier.props.accessibilityHint).toContain(
      w.signIn.identifierHintCountries.replace('{codes}', '+1'),
    );
    expect(screen.getByRole('header', { name: w.signIn.title })).toBeTruthy();
  });

  test('the profile: what is missing is said, and stays with its field', async () => {
    await open('/', { signedIn: false });
    await screen.findByText(w.signIn.intro);
    await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), 'ana@example.test');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.sendCode }));
    await screen.findByLabelText(w.signIn.codeLabel);
    expect(audit()).toEqual([]);
    await fireEvent.changeText(screen.getByLabelText(w.signIn.codeLabel), '123456');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.submit }));
    await screen.findByText(w.profile.firstIntro);
    expect(audit()).toEqual([]);

    await fireEvent.press(screen.getByRole('button', { name: w.profile.continue }));
    await screen.findByText(w.profile.nameRequired);
    expect(announced).toHaveBeenCalledWith(w.profile.nameRequired);
    expect(announced).toHaveBeenCalledWith(w.profile.adultRequired);
    expect(screen.getByLabelText(w.profile.nameLabel).props.accessibilityHint).toContain(
      w.profile.nameRequired,
    );
    const adult = screen.getByLabelText(w.profile.adultLabel);
    expect(adult.props.accessibilityState).toMatchObject({ checked: false });
    expect(audit()).toEqual([]);
  });
});

describe('an invitation, before signing in', () => {
  test('the first screen offers it, as a button that says what it opens', async () => {
    await open('/', { signedIn: false });
    await screen.findByText(w.mobile.invited.heading);
    expect(audit()).toEqual([]);
    expect(screen.getByRole('header', { name: w.mobile.invited.heading })).toBeTruthy();
    expect(screen.getByRole('button', { name: w.mobile.openInvitation.title })).toBeTruthy();
  });
});

describe('the invitation, as it opens from a link', () => {
  test('signing in, reading the proposal and setting up to respond', async () => {
    await open(`/en/i#${INVITATION}`, { signedIn: false });
    await screen.findByText(w.invitation.signInToRead);
    expect(screen.getByRole('header', { name: w.invitation.signedOutTitle })).toBeTruthy();
    expect(screen.getByRole('header', { name: w.signIn.title })).toBeTruthy();
    expect(audit()).toEqual([]);

    await signInOnScreen(w);
    await screen.findByText(w.invitation.notBinding);
    expect(screen.getByRole('header', { name: w.invitation.title })).toBeTruthy();
    expect(audit()).toEqual([]);

    await fireEvent.press(screen.getByRole('button', { name: w.invitation.respondNew }));
    await screen.findByText(w.profile.firstIntro);
    expect(screen.getByRole('header', { name: w.profile.firstTitle })).toBeTruthy();
    expect(audit()).toEqual([]);
  });
});

describe('the exchanges', () => {
  test('each one is a button that says what it is and that it opens', async () => {
    await open('/', { signedIn: true });
    await screen.findByText('With Ben Ortiz');
    expect(audit()).toEqual([]);
    const card = screen.getByRole('button', { name: /With Ben Ortiz/ });
    expect(card.props.accessibilityLabel).toContain('Reference PVVS-5Q2K');
    expect(card.props.accessibilityHint).toBe(w.a11y.openExchange);
  });

  test('a name written to sound like a status is said after the real one', async () => {
    // The other party chose this name; it must not be the first thing said.
    const spoof = 'Sam. Completed. Reference EX-0001.\u202E\n';
    await open('/', {
      signedIn: true,
      prepare: (fake) => {
        fake.otherPartyName = spoof;
      },
    });
    await screen.findByText(/^With Sam/);
    const card = screen.getByRole('button', { name: /Reference PVVS-5Q2K/ });
    const label: string = card.props.accessibilityLabel;
    expect(label.startsWith(w.states.ACTIVE + '. Reference PVVS-5Q2K. ')).toBe(true);
    expect(label.endsWith('. With Sam. Completed. Reference EX-0001.')).toBe(true);
    expect(label).not.toMatch(/[\u0000-\u001f\u202A-\u202E\u2066-\u2069]/);
  });
});

describe('notifications', () => {
  afterEach(resetNotifications);
  const withPush = (fake: FakeService) => {
    notifications.projectId = 'test-project-id';
    fake.push = true;
  };

  test('the offer on the list is headed, and each of its buttons named', async () => {
    await open('/', { signedIn: true, prepare: withPush });
    await screen.findByText(w.mobile.notifications.askHeading);
    expect(audit()).toEqual([]);
    screen.getByRole('button', { name: w.mobile.notifications.turnOn });
    screen.getByRole('button', { name: w.mobile.notifications.notNow });
  });

  test('the switch on the account screen is labelled, says what it does, and its state', async () => {
    await open('/account', { signedIn: true, prepare: withPush });
    const toggle = await screen.findByLabelText(w.mobile.notifications.switch);
    expect(audit()).toEqual([]);
    expect(toggle.props.accessibilityRole).toBe('switch');
    expect(toggle.props.accessibilityHint).toBe(w.mobile.notifications.switchHint);
    expect(toggle.props.accessibilityState).toMatchObject({ checked: false });
  });

  test('refused in the system settings, the way there is a named button', async () => {
    await open('/account', {
      signedIn: true,
      prepare: (fake) => {
        withPush(fake);
        notifications.permission = { granted: false, canAskAgain: false };
      },
    });
    await screen.findByText(w.mobile.notifications.blocked);
    expect(audit()).toEqual([]);
    screen.getByRole('button', { name: w.mobile.notifications.openSettings });
  });
});

describe('the composer', () => {
  test('writing, and what needs fixing said once', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    await screen.findByText(w.composer.titleFirst);
    expect(audit()).toEqual([]);
    expect(screen.getByLabelText(w.composer.yourName).props.accessibilityHint).toContain(
      w.a11y.required,
    );

    await fireEvent.changeText(screen.getByLabelText(w.composer.descriptionLabel), '');
    announced.mockClear();
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    const summary = '1 thing needs fixing before you can sign.';
    await screen.findByText(summary);
    // The count is said; each field's error is read when its field is reached.
    expect(announced.mock.calls.map(([text]) => text)).toEqual([summary]);
    expect(screen.getByLabelText(w.composer.descriptionLabel).props.accessibilityHint).toContain(
      w.composer.problems.DESCRIPTION_MISSING,
    );
    expect(audit()).toEqual([]);
  });

  test('the signing step says why its button cannot be pressed yet', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    await screen.findByText(w.composer.titleFirst);
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await screen.findByText(w.composer.signIntro);
    expect(audit()).toEqual([]);

    const sign = screen.getByTestId('consent-sign');
    expect(sign.props.accessibilityState).toMatchObject({ disabled: true });
    expect(sign.props.accessibilityHint).toBe(w.a11y.signNeedsAgreement);
    await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
    expect(screen.getByTestId('consent-sign').props.accessibilityHint).toBeUndefined();
    expect(screen.getByTestId('consent-agree').props.accessibilityState).toMatchObject({
      checked: true,
    });
  });
});

describe('passing the invitation on', () => {
  test('who it is for is a labelled field, and what is wrong with it is said with it', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    const field = await screen.findByLabelText(w.invitationLink.forLabel);
    expect(field.props.accessibilityHint).toContain(w.invitationLink.forHint);
    await fireEvent.changeText(field, 'carla@');
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await screen.findByText(w.invitationLink.forInvalid);
    expect(screen.getByLabelText(w.invitationLink.forLabel).props.accessibilityHint).toContain(
      w.invitationLink.forInvalid,
    );
    expect(audit()).toEqual([]);
  });

  test('the link once sent, its buttons named, and the QR code an image with a name', async () => {
    await open(`/exchanges/${DRAFT}`, { signedIn: true });
    await screen.findByText(w.composer.titleFirst);
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await screen.findByText(w.composer.signIntro);
    await fireEvent(screen.getByTestId('consent-agree'), 'valueChange', true);
    await fireEvent.press(screen.getByTestId('consent-sign'));
    await screen.findByText(w.invitationLink.intro);
    expect(audit()).toEqual([]);

    await fireEvent.press(screen.getByRole('button', { name: w.invitationLink.shareQr }));
    const code = screen.getByRole('image', { name: w.invitationLink.qrLabel });
    expect(code.props.accessible).toBe(true);
    expect(
      screen.getByRole('button', { name: w.invitationLink.hideQr }).props.accessibilityState,
    ).toMatchObject({ expanded: true });
    expect(audit()).toEqual([]);
  });

  test('waiting for a code: where to look is said, and another is offered later', async () => {
    await open('/', { signedIn: false });
    await screen.findByText(w.signIn.intro);
    await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), 'ana@example.test');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.sendCode }));
    await screen.findByLabelText(w.signIn.codeLabel);
    screen.getByText(w.signIn.resendSoon);
    expect(audit()).toEqual([]);
  });
});

describe('the exchange', () => {
  test('a panel takes the screen reader to it, and gives the focus back on cancel', async () => {
    await open(`/exchanges/${EXCHANGE}`, { signedIn: true });
    await screen.findByText('Yup with Ben Ortiz');
    // The device's own wallet button is there to be audited too.
    await screen.findByRole('button', {
      name: Platform.OS === 'ios' ? w.wallet.addToApple : w.wallet.addToGoogle,
    });
    expect(audit()).toEqual([]);

    const opener = screen.getByRole('button', { name: w.exchange.moves.CLAIM });
    await fireEvent.press(opener);
    expect(
      screen.getByRole('button', { name: w.exchange.moves.CLAIM, expanded: true }),
    ).toBeTruthy();
    await waitFor(() => expect(focused).toHaveBeenCalledWith(expect.anything(), 'focus'));
    expect(audit()).toEqual([]);

    focused.mockClear();
    await fireEvent.press(screen.getByRole('button', { name: w.common.cancel }));
    await waitFor(() => expect(focused).toHaveBeenCalledWith(expect.anything(), 'focus'));
  });

  test('a panel’s heading sits under the part of the screen it opens in', async () => {
    await open(`/exchanges/${EXCHANGE}`, { signedIn: true });
    await screen.findByText('Yup with Ben Ortiz');
    const level = (name: string) =>
      screen.getAllByRole('header', { name }).map((node) => node.props['aria-level']);
    expect(level('Yup with Ben Ortiz')).toEqual([1]);
    expect(level(w.safety.heading)).toEqual([2]);

    const block = w.safety.block.replace('{name}', 'Ben Ortiz');
    await fireEvent.press(screen.getByRole('button', { name: block }));
    // The panel, under “Report or block”.
    expect(level(block)).toEqual([3]);
    expect(audit()).toEqual([]);
  });

  test('the heading, said first, has nothing in the name that turns it around', async () => {
    await open(`/exchanges/${EXCHANGE}`, {
      signedIn: true,
      prepare: (fake) => {
        const revision = fake.exchange.in_force_revision!;
        fake.exchange = {
          ...fake.exchange,
          in_force_revision: {
            ...revision,
            terms: { ...revision.terms, party_b_name: 'Ben\u202E\n Ortiz' },
          },
        };
      },
    });
    expect(await screen.findByRole('header', { name: 'Yup with Ben Ortiz' })).toBeTruthy();
  });

  test('what an action did is said', async () => {
    await open(`/exchanges/${EXCHANGE}`, { signedIn: true });
    await screen.findByText('Yup with Ben Ortiz');
    await fireEvent.press(screen.getByRole('button', { name: w.exchange.moves.CLAIM }));
    await fireEvent.press(screen.getAllByRole('button', { name: w.exchange.moves.CLAIM }).at(-1)!);
    await screen.findByText(w.exchange.updated);
    expect(announced).toHaveBeenCalledWith(w.exchange.updated);
  });
});

describe('the record', () => {
  test('laid out to be read from top to bottom, its plain summary first', async () => {
    await open(`/exchanges/${EXCHANGE}/record`, { signedIn: true });
    await screen.findByText(w.mobile.record.share);
    await screen.findByText(w.record.summary.heading);
    expect(audit()).toEqual([]);
    expect(screen.getByRole('header', { name: w.record.summary.heading })).toBeTruthy();
    expect(screen.getByRole('button', { name: w.mobile.record.savePdf })).toBeTruthy();
  });
});

describe('when something isn’t working', () => {
  test('the guide: its question, then the ways forward, each a named button', async () => {
    await open(`/exchanges/${EXCHANGE}`, { signedIn: true });
    await screen.findByText('Yup with Ben Ortiz');
    const opener = screen.getByRole('button', { name: w.trouble.open });
    await fireEvent.press(opener);
    expect(screen.getByRole('button', { name: w.trouble.open, expanded: true })).toBeTruthy();
    await waitFor(() => expect(focused).toHaveBeenCalledWith(expect.anything(), 'focus'));
    expect(audit()).toEqual([]);

    focused.mockClear();
    await fireEvent.press(
      screen.getByRole('button', { name: w.trouble.situations.THEY_HAVENT.replace('{name}', 'Ben Ortiz') }),
    );
    // The screen reader is taken to what the situation means.
    await waitFor(() => expect(focused).toHaveBeenCalledWith(expect.anything(), 'focus'));
    expect(audit()).toEqual([]);
    // An action offered on an item says which item it is.
    const waive = screen
      .getAllByRole('button', { name: w.exchange.moneyMoves.WAIVE })
      .map((button) => button.props.accessibilityHint);
    expect(waive).toContain('Payment for the repair');
  });

  test('a disputed item says it is recorded, not decided', async () => {
    await open(`/exchanges/${EXCHANGE}`, {
      signedIn: true,
      prepare: (fake) => {
        fake.exchange = {
          ...fake.exchange,
          contributions: [
            { id: REPAIR, status: 'DISPUTED' },
            { id: PAYMENT, status: 'PENDING' },
          ],
        };
      },
    });
    await screen.findByText(w.dispute.weRecord);
    expect(audit()).toEqual([]);
  });
});

describe('a proposal waiting to be signed', () => {
  test('what it changes, above the way to sign', async () => {
    await open(`/exchanges/${EXCHANGE}`, {
      signedIn: true,
      prepare: (fake) => {
        const inForce = fake.exchange.in_force_revision!;
        fake.exchange = {
          ...fake.exchange,
          open_revision: {
            ...inForce,
            id: 'c0000000-0000-4000-8000-000000000002',
            sequence: 2,
            author: 'B',
            accepted_by: ['B'],
            terms: {
              ...inForce.terms,
              contributions: [
                { ...inForce.terms.contributions[0], description: 'Repair the fence and gate' },
                inForce.terms.contributions[1],
              ],
            },
          },
        };
      },
    });
    await screen.findByText(w.proposalChanges.heading);
    expect(audit()).toEqual([]);
    expect(screen.getByRole('header', { name: w.proposalChanges.heading })).toBeTruthy();
  });
});
