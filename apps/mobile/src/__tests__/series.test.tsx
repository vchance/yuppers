import { wordingFor } from '@yuppers/shared';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';
import { AccessibilityInfo } from 'react-native';

import { forgetInvitation } from '../lib/invitation';
import {
  DRAFT,
  EXCHANGE,
  fakeService,
  INSTALMENT_THREE,
  INSTALMENT_TWO,
  REPAIR,
  seriesExchange,
  TOKEN,
  ana,
  type FakeService,
} from './fake-service';

/*
 * Instalments and stages (DESIGN.md §7.1, §7.2) on the device: the two split
 * sheets in the composer, the counts in words on the yup, in the list and in
 * the record, "Mark the rest as paid", and progress notes.
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
const split = w.composer.split;

async function open(initialUrl: string, prepare?: (service: FakeService) => void) {
  mockKeychain.clear();
  service = fakeService();
  mockKeychain.set('yuppers.session', TOKEN);
  service.account = ana;
  prepare?.(service);
  await renderRouter('src/app', { initialUrl });
}

let announced: jest.SpyInstance;
beforeEach(() => {
  announced = jest.spyOn(AccessibilityInfo, 'announceForAccessibility');
});
afterEach(() => {
  forgetInvitation();
  jest.restoreAllMocks();
});

const commands = () =>
  service.sent
    .filter((request) => request.path.endsWith('/commands'))
    .map((request) => (request.body as { command: { type: string } }).command);

describe('the instalments sheet', () => {
  async function openSheet() {
    await open(`/exchanges/${DRAFT}`);
    await fireEvent.press(await screen.findByRole('radio', { name: w.contributionTypes.MONEY }));
    await fireEvent.changeText(screen.getByLabelText('Amount in USD'), '100');
    await fireEvent.press(screen.getByText(split.instalments.link));
  }

  test('shows every amount and date before anything is added, then adds the payments', async () => {
    await openSheet();
    await screen.findByText(split.instalments.previewHeading);
    expect(screen.getAllByText(/: \$33\.33, due /)).toHaveLength(2);
    screen.getByText(/ 3 of 3: \$33\.34, due /);
    screen.getByText(split.instalments.plainHint);
    // Nothing is added until it is asked for.
    expect(screen.queryByRole('button', { name: 'Remove item 2' })).toBeNull();

    await fireEvent.press(screen.getByText(split.instalments.done));
    await screen.findByRole('button', { name: 'Remove item 3' });
    expect(screen.queryByRole('button', { name: 'Remove item 4' })).toBeNull();
    expect(announced).toHaveBeenCalledWith('3 payments added.');
    screen.getByDisplayValue('Repair the back fence 2 of 3');
  });

  test('an amount that is missing is said, and adds nothing', async () => {
    await openSheet();
    await fireEvent.changeText(screen.getByLabelText('Amount to share out, in USD'), '');
    await fireEvent.press(screen.getByText(split.instalments.done));
    await screen.findByText('Enter an amount, such as 25.50.');
    expect(screen.queryByRole('button', { name: 'Remove item 2' })).toBeNull();
  });

  test('one press puts it back as one payment', async () => {
    await openSheet();
    await fireEvent.press(screen.getByText(split.instalments.done));
    await fireEvent.press(await screen.findByText(split.instalments.undo));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Remove item 2' })).toBeNull());
    expect(announced).toHaveBeenCalledWith(split.instalments.undone);
    screen.getByDisplayValue('100');
  });
});

describe('the stages sheet', () => {
  test('names each stage, can add a payment for each, and previews them all', async () => {
    await open(`/exchanges/${DRAFT}`);
    await fireEvent.press(await screen.findByText(split.stages.link));
    // Examples sit under the label as hints, never as placeholders.
    const first = screen.getByLabelText('Name of stage 1');
    expect(first.props.accessibilityHint).toBe(split.stages.exampleOne);
    expect(first.props.placeholder).toBeUndefined();
    await fireEvent.changeText(screen.getByLabelText('Name of stage 1'), 'Posts set');
    await fireEvent.changeText(screen.getByLabelText('Name of stage 2'), 'Panels up');
    await fireEvent.changeText(screen.getByLabelText('Name of stage 3'), 'Painted');
    await fireEvent(screen.getByLabelText(split.stages.payLabel), 'valueChange', true);
    await fireEvent.changeText(screen.getByLabelText('Amount to share out, in USD'), '90');
    await screen.findByText(/Payment for Posts set: \$30\.00, due once stage 1 is confirmed/);
    screen.getByText('Stage 2: Panels up, due when the agreement is signed');

    await fireEvent.press(screen.getByText(split.stages.done));
    await screen.findByRole('button', { name: 'Remove item 6' });
    expect(announced).toHaveBeenCalledWith('6 items added.');
    await fireEvent.press(screen.getByText(split.stages.undo));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Remove item 2' })).toBeNull());
  });

  test('every stage has to be named', async () => {
    await open(`/exchanges/${DRAFT}`);
    await fireEvent.press(await screen.findByText(split.stages.link));
    await fireEvent.press(screen.getByText(split.stages.done));
    await screen.findAllByText(split.stages.problems.NAME);
    expect(screen.queryByRole('button', { name: 'Remove item 2' })).toBeNull();
  });
});

describe('counts, in words', () => {
  test('on the yup, with the rest to mark as paid', async () => {
    await open(`/exchanges/${EXCHANGE}`, (fake) => {
      fake.exchange = seriesExchange();
    });
    await screen.findByText('You: 1 of 3 payments confirmed');
    await fireEvent.press(screen.getByRole('button', { name: w.exchange.markRest.button }));
    screen.getByText(/records that you paid Ben Ortiz 2 payments/);
    await fireEvent.press(screen.getByRole('button', { name: 'Record 2 payments as paid' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        type: 'CLAIM_REST',
        contributions: [INSTALMENT_TWO, INSTALMENT_THREE],
      }),
    );
    expect(announced).toHaveBeenCalledWith('2 payments recorded as paid.');
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: w.exchange.markRest.button })).toBeNull(),
    );
  });

  test('in the list', async () => {
    await open('/', (fake) => {
      fake.exchange = seriesExchange();
    });
    await screen.findByText('1 of 3 payments confirmed');
  });

  test('in the record’s summary', async () => {
    await open(`/exchanges/${EXCHANGE}/record`, (fake) => {
      fake.exchange = seriesExchange();
    });
    await screen.findByText('Ana Ruiz: 1 of 3 payments confirmed');
  });
});

describe('progress notes', () => {
  test('the provider writes one; it shows under the item and in the history, and the status stays', async () => {
    await open(`/exchanges/${EXCHANGE}`);
    await fireEvent.press(await screen.findByTestId(`progress-${REPAIR}`));
    // Nothing written: asked for.
    await fireEvent.press(screen.getByRole('button', { name: w.exchange.progress.send }));
    await screen.findByText(w.exchange.progress.required);

    await fireEvent.changeText(screen.getByLabelText(w.exchange.progress.label), 'Posts are in.');
    await fireEvent.press(screen.getByRole('button', { name: w.exchange.progress.send }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        type: 'NOTE_PROGRESS',
        contribution: REPAIR,
        note: 'Posts are in.',
      }),
    );
    expect(announced).toHaveBeenCalledWith(w.exchange.progress.added);
    await screen.findByText('1 progress note');
    // Under the item, and again in the history with its own label.
    expect((await screen.findAllByText('Posts are in.')).length).toBeGreaterThanOrEqual(2);
    screen.getByText(w.record.noteLabels.progress);
    expect(service.exchange.contributions.find((item) => item.id === REPAIR)?.status).toBe('PENDING');
  });

  test('a full item says so', async () => {
    await open(`/exchanges/${EXCHANGE}`, (fake) => {
      fake.progressFull = true;
    });
    await fireEvent.press(await screen.findByTestId(`progress-${REPAIR}`));
    await fireEvent.changeText(screen.getByLabelText(w.exchange.progress.label), 'More.');
    await fireEvent.press(screen.getByRole('button', { name: w.exchange.progress.send }));
    await screen.findByText(w.exchange.progress.full);
  });

  test('only the provider’s own items offer it', async () => {
    await open(`/exchanges/${EXCHANGE}`);
    await screen.findByTestId(`progress-${REPAIR}`);
    expect(screen.getAllByText(w.exchange.progress.add)).toHaveLength(1);
  });
});
