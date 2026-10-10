import { TEMPLATES, wordingFor } from '@yuppers/shared';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';

import { forgetInvitation } from '../lib/invitation';
import { DRAFT, TOKEN, ana, fakeService, type FakeService } from './fake-service';

/*
 * Starting a yup and the sample yup (DESIGN.md sections 4.3 and 4.4), run as
 * the app: New yup opens on the common agreements, the blank form and
 * copying an earlier yup, and choosing one makes the draft with what it
 * puts in; the example is a read-only record anyone can open.
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

let service: FakeService = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = wordingFor('en');

async function open(
  initialUrl: string,
  { signedIn = true, prepare }: { signedIn?: boolean; prepare?: (service: FakeService) => void } = {},
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

afterEach(forgetInvitation);

/** What the app asked of the service to start a yup, in order. */
function startedBy() {
  return service.sent.filter(
    (request) =>
      (request.method === 'POST' && request.path === '/v1/exchanges') ||
      (request.method === 'PUT' && request.path === `/v1/exchanges/${DRAFT}/draft`),
  );
}

describe('the chooser', () => {
  test('New yup opens on the common agreements, then blank and copy, and what yups are not for', async () => {
    await open('/');
    await screen.findByText(w.home.groupOpen);
    await fireEvent.press(screen.getByRole('button', { name: w.home.start }));
    await screen.findByRole('header', { name: w.templates.chooserTitle });

    // The six, in the order the design gives, then blank and copy, each a button named for it.
    for (const template of TEMPLATES) {
      expect(
        screen.getByRole('button', { name: w.templates.entries[template.id].name }),
      ).toBeTruthy();
    }
    screen.getByRole('button', { name: w.templates.blank.name });
    screen.getByRole('button', { name: w.templates.copy.name });
    // The hint is in words, and the line about what yups are not for.
    screen.getByText(w.templates.entries['selling-something'].summary);
    screen.getByText(w.templates.notFor);
    screen.getByText(w.templates.notForAdvice);
    // New Jersey's warning is on the job and on no other; lending money is not there.
    screen.getByText(w.templates.entries['job-deposit-balance'].warning!);
    expect(screen.queryAllByText(/New Jersey/)).toHaveLength(1);
    expect(screen.queryByText(/loan|lend money|lending money/i)).toBeNull();
  });

  test('a common agreement makes the draft from it and opens it with examples in grey', async () => {
    await open('/new');
    await screen.findByRole('header', { name: w.templates.chooserTitle });
    await fireEvent.press(
      screen.getByRole('button', { name: w.templates.entries['job-deposit-balance'].name }),
    );
    await screen.findByRole('header', { name: w.composer.titleFirst });

    const [created, saved] = startedBy();
    // The service is told once what the draft was started from; the working
    // copy has no trace of it.
    expect(created.body).toMatchObject({ timezone: 'America/Chicago', started_from: 'job-deposit-balance@1' });
    expect(JSON.stringify(saved.body)).not.toContain('job-deposit-balance');

    const entry = w.templates.entries['job-deposit-balance'];
    for (const item of entry.items) {
      expect(screen.getAllByPlaceholderText(item.description).length).toBeGreaterThanOrEqual(1);
    }
    screen.getByText(entry.hint);
    screen.getByText(w.templates.bandExamples);
    screen.getByText(entry.warning!);
  });

  test('an example counts as empty: the composer asks for a description', async () => {
    await open('/new');
    await screen.findByRole('header', { name: w.templates.chooserTitle });
    await fireEvent.press(
      screen.getByRole('button', { name: w.templates.entries['swap-no-money'].name }),
    );
    await screen.findByRole('header', { name: w.composer.titleFirst });
    await fireEvent.press(screen.getByRole('button', { name: w.composer.review }));
    await waitFor(() =>
      expect(screen.getAllByText(w.composer.problems.DESCRIPTION_MISSING)).toHaveLength(2),
    );
  });

  test('swap sides turns every item round', async () => {
    await open('/new');
    await screen.findByRole('header', { name: w.templates.chooserTitle });
    await fireEvent.press(
      screen.getByRole('button', { name: w.templates.entries['selling-something'].name }),
    );
    await screen.findByRole('header', { name: w.composer.titleFirst });
    const provided = () =>
      screen
        .getAllByRole('radio', { checked: true })
        .map((node) => node.props.accessibilityLabel)
        .filter((label: string) => label === w.party.you || label === w.party.other);
    expect(provided()).toEqual([w.party.you, w.party.other]);
    await fireEvent.press(screen.getByRole('button', { name: w.templates.swapSides }));
    await waitFor(() => expect(provided()).toEqual([w.party.other, w.party.you]));
  });

  test('blank is the empty composer, and the service is told so', async () => {
    await open('/new');
    await screen.findByRole('header', { name: w.templates.chooserTitle });
    await fireEvent.press(screen.getByRole('button', { name: w.templates.blank.name }));
    await screen.findByRole('header', { name: w.composer.titleFirst });
    const [created, ...rest] = startedBy();
    expect(created.body).toMatchObject({ started_from: 'blank' });
    expect(rest.filter((request) => request.method === 'PUT')).toHaveLength(0);
    expect(screen.queryByText(w.templates.bandExamples)).toBeNull();
  });
});

describe('copying a previous yup', () => {
  test('asks who the copy is for, starting at someone else, and copies items and terms', async () => {
    await open('/new');
    await screen.findByRole('header', { name: w.templates.chooserTitle });
    await fireEvent.press(screen.getByRole('button', { name: w.templates.copy.name }));
    await screen.findByRole('header', { name: w.templates.copyHeading });
    await fireEvent.press(
      await screen.findByRole('button', { name: w.templates.copyStartNamed.replace('{name}', 'Ben Ortiz') }),
    );
    screen.getByRole('radio', { name: w.templates.copyForSomeoneElse, checked: true });
    await fireEvent.press(screen.getByRole('button', { name: w.templates.copyStart }));
    await screen.findByRole('header', { name: w.composer.titleFirst });

    const [created, saved] = startedBy();
    expect(created.body).toMatchObject({ started_from: 'copy' });
    const draft = (
      saved.body as {
        body: { partyA: string; partyB: string; terms: string; contributions: any[] };
      }
    ).body;
    expect(draft.partyB).toBe('');
    expect(draft.terms).toBe('Repair the back fence.');
    expect(draft.contributions[0].due).toEqual({ kind: 'DATE', date: '' });
    expect(draft.contributions[1].due).toMatchObject({ kind: 'AFTER_CONTRIBUTION' });
  });

  test('says so when there is nothing to copy', async () => {
    await open('/new', {
      prepare: (fake) => {
        fake.noExchanges = true;
      },
    });
    await screen.findByRole('header', { name: w.templates.chooserTitle });
    await fireEvent.press(screen.getByRole('button', { name: w.templates.copy.name }));
    await screen.findByText(w.templates.copyNone);
  });
});

describe('the sample yup', () => {
  test('is readable signed out, says it is an example, and asks the service nothing about yups', async () => {
    await open('/example', { signedIn: false });
    await screen.findByRole('header', { name: w.sample.title });
    screen.getByText(w.sample.banner);
    screen.getByText(w.sample.terms);
    screen.getByText(w.sample.fingerprint);
    expect(
      service.sent.filter(
        (request) =>
          request.path.includes('/v1/exchanges') || request.path.includes('/v1/invitations'),
      ),
    ).toEqual([]);
  });

  test('shows each action by its own name, disabled, and says why', async () => {
    await open('/example', { signedIn: false });
    await screen.findByRole('header', { name: w.sample.title });
    for (const name of [w.exchange.moves.CLAIM, w.exchange.moneyMoves.CLAIM]) {
      const action = screen.getByRole('button', { name });
      expect(action.props.accessibilityState).toMatchObject({ disabled: true });
      expect(action.props.accessibilityHint).toBe(w.sample.actionsOff);
    }
    screen.getByText(new RegExp(w.sample.waitsOn));
    // Nothing that signs, confirms, disputes or reports.
    expect(screen.queryByRole('button', { name: w.exchange.moves.CONFIRM })).toBeNull();
    expect(screen.queryByRole('button', { name: w.exchange.moves.DISPUTE })).toBeNull();
    expect(screen.queryByRole('button', { name: w.exchange.accept })).toBeNull();
  });

  test('the empty home screen has a card that opens it, which goes once there are yups', async () => {
    await open('/', {
      prepare: (fake) => {
        fake.noExchanges = true;
      },
    });
    await screen.findByText(w.sample.homeHeading);
    await fireEvent.press(screen.getByRole('button', { name: w.sample.see }));
    await screen.findByRole('header', { name: w.sample.title });
  });

  test('is not on the home screen once the person has a yup', async () => {
    await open('/');
    await screen.findByText(w.home.groupOpen);
    expect(screen.queryByText(w.sample.homeHeading)).toBeNull();
  });

  test('the sign-in screen has a line that opens it', async () => {
    await open('/', { signedIn: false });
    await fireEvent.press(await screen.findByRole('link', { name: w.sample.signInLine }));
    await screen.findByRole('header', { name: w.sample.title });
  });
});

describe('in Spanish', () => {
  test('the chooser and the example are in Spanish, with the same people', async () => {
    const es = wordingFor('es');
    await open('/new', {
      prepare: (fake) => {
        fake.account = { ...ana, language: 'es' };
      },
    });
    await screen.findByRole('header', { name: es.templates.chooserTitle });
    screen.getByText(es.templates.notFor);
    screen.getByText(es.templates.entries['job-deposit-balance'].warning!);
  });
});
