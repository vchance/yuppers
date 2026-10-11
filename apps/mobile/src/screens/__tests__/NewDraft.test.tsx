import type { Account, ExchangeView } from '@yuppers/api-client';
import { createI18n, wordingFor } from '@yuppers/shared';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';

import { I18nContext, SessionContext, type Session } from '../../lib/context';
import { api } from '../../lib/session';
import { NewDraftScreen } from '../ExchangeScreen';

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => {
  let count = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String(++count).padStart(12, '0')}` };
});

const mockReplace = jest.fn();
const mockDismissTo = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ replace: mockReplace, dismissTo: mockDismissTo }),
}));

const wording = wordingFor('en');
const i18n = createI18n('en', wording, () => {});

const ID = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70';
const made = {
  id: ID,
  version: 1,
  state: 'DRAFT',
  you: 'A',
  counterparty: 'UNCLAIMED',
  currency: 'USD',
  timezone: 'America/Chicago',
  display_code: 'ABCD-1234',
  contributions: [],
} as ExchangeView;

const session = {
  ready: true,
  account: { display_name: 'Ana Ruiz' } as Account,
} as Session;

function open(from: string) {
  return render(
    <SessionContext value={session}>
      <I18nContext value={i18n}>
        <NewDraftScreen from={from} />
      </I18nContext>
    </SessionContext>,
  );
}

beforeEach(() => {
  jest.useFakeTimers();
  jest.spyOn(api, 'createExchange').mockResolvedValue(made);
  jest.spyOn(api, 'saveDraft').mockResolvedValue(undefined as never);
  jest.spyOn(api, 'meta').mockRejectedValue(new Error('offline'));
});

afterEach(() => {
  jest.useRealTimers();
  jest.restoreAllMocks();
  mockReplace.mockReset();
  mockDismissTo.mockReset();
});

test('a blank start that is left alone makes nothing on the service', async () => {
  const view = open('blank');
  await act(async () => {
    jest.advanceTimersByTime(5000);
  });
  expect(screen.getByText(wording.composer.titleFirst)).toBeTruthy();
  view.unmount();
  await act(async () => {
    jest.advanceTimersByTime(5000);
  });
  expect(api.createExchange).not.toHaveBeenCalled();
  expect(api.saveDraft).not.toHaveBeenCalled();
  expect(mockReplace).not.toHaveBeenCalled();
});

test('the first change makes the draft, tells the service how it began, saves, and moves to it', async () => {
  open('selling-something');
  // The band for the template is there before anything is made.
  expect(screen.getByText(wording.templates.bandExamples)).toBeTruthy();
  expect(api.createExchange).not.toHaveBeenCalled();

  await fireEvent.changeText(screen.getByLabelText(wording.composer.otherName), 'Ben');
  await act(async () => {
    jest.advanceTimersByTime(2000);
  });
  await waitFor(() => expect(mockReplace).toHaveBeenCalledWith(`/exchanges/${ID}`));
  expect(api.createExchange).toHaveBeenCalledTimes(1);
  expect(api.createExchange).toHaveBeenCalledWith(expect.any(String), 'selling-something@1');
  expect(api.saveDraft).toHaveBeenCalledWith(ID, expect.objectContaining({ partyB: 'Ben' }));
});
