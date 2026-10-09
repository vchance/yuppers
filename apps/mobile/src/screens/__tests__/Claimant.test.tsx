import type { Command, ExchangeView } from '@yuppers/api-client';
import { ApiFailure, createI18n, wordingFor, type Actions } from '@yuppers/shared';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { useState } from 'react';

import { I18nContext } from '../../lib/context';
import { api } from '../../lib/session';
import { ClaimantWaiting, ConfirmClaimant } from '../Claimant';
import { ExchangeSafety } from '../ExchangeSafety';

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));

const mockDismissTo = jest.fn();
jest.mock('expo-router', () => ({ useRouter: () => ({ dismissTo: mockDismissTo }) }));

const wording = wordingFor('en');
const i18n = createI18n('en', wording, () => {});
const c = wording.claimant;
const fill = (message: string, name: string) => message.replaceAll('{name}', name);

const ID = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70';
const sam = { display_name: 'Sam Stranger', identifier: 's•••@example.test' };

function exchange(patch: Partial<ExchangeView>): ExchangeView {
  return {
    id: ID,
    version: 3,
    state: 'NEGOTIATING',
    you: 'A',
    counterparty: 'CLAIMED',
    currency: 'USD',
    timezone: 'America/Chicago',
    display_code: 'ABCD-1234',
    contributions: [],
    claimant: sam,
    open_revision: {
      id: 'open',
      sequence: 1,
      author: 'A',
      expires_at: '2026-10-20T15:00:00Z',
      accepted_by: ['A'],
      content_hash: 'ab'.repeat(32),
      terms: { party_a_name: 'Ana Ruiz', party_b_name: 'Ben Ortiz', terms: '', contributions: [] },
    },
    ...patch,
  };
}

/** The exchange screen's action runner, with panels that open and close as there. */
function useFakeActions(run: (command: Command) => Promise<boolean>): Actions {
  const [panel, setPanel] = useState<string | null>(null);
  return {
    busy: false,
    failure: null,
    done: false,
    panel,
    open: setPanel,
    close: () => setPanel(null),
    run,
  };
}

function Confirming({ run, onRejected }: { run: Actions['run']; onRejected(): void }) {
  const actions = useFakeActions(run);
  return (
    <ConfirmClaimant
      exchange={exchange({})}
      claimant={sam}
      actions={actions}
      onRejected={onRejected}
    />
  );
}

function Waiting({ signed }: { signed: boolean }) {
  const actions = useFakeActions(async () => true);
  return (
    <ClaimantWaiting
      exchange={exchange({
        you: 'B',
        claimant: null,
        open_revision: {
          ...exchange({}).open_revision!,
          accepted_by: signed ? ['A', 'B'] : ['A'],
        },
      })}
      otherName="Ana Ruiz"
      actions={actions}
    />
  );
}

const show = (ui: React.ReactElement) => render(<I18nContext value={i18n}>{ui}</I18nContext>);

afterEach(() => {
  jest.restoreAllMocks();
  mockDismissTo.mockReset();
  mockReload.mockClear();
});

test('the initiator can confirm whoever opened the link, in one press', async () => {
  const run = jest.fn(async () => true);
  await show(<Confirming run={run} onRejected={() => {}} />);

  expect(screen.getByText(/Sam Stranger, s•••@example.test/)).toBeTruthy();
  await fireEvent.press(screen.getByText(wording.exchange.confirmCounterparty));
  expect(run).toHaveBeenCalledWith({ type: 'CONFIRM_COUNTERPARTY' });
});

test('saying "not who I invited" is explained first and takes a second, deliberate press', async () => {
  const run = jest.fn(async () => true);
  const onRejected = jest.fn();
  await show(<Confirming run={run} onRejected={onRejected} />);

  await fireEvent.press(screen.getByText(c.reject));
  expect(run).not.toHaveBeenCalled();
  for (const text of [
    fill(c.rejectRemoves, 'Sam Stranger'),
    c.rejectVoids,
    c.rejectKeeps,
    c.rejectQuiet,
  ]) {
    expect(screen.getByText(text)).toBeTruthy();
  }

  await fireEvent.press(screen.getByText(c.confirmReject));
  expect(run).toHaveBeenCalledWith({ type: 'REJECT_COUNTERPARTY' });
  await waitFor(() => expect(onRejected).toHaveBeenCalledTimes(1));
});

test('changing one’s mind about removing them sends nothing', async () => {
  const run = jest.fn(async () => true);
  await show(<Confirming run={run} onRejected={() => {}} />);

  await fireEvent.press(screen.getByText(c.reject));
  await fireEvent.press(screen.getByText(wording.common.cancel));
  expect(screen.queryByText(c.confirmReject)).toBeNull();
  expect(run).not.toHaveBeenCalled();
});

test('a refused removal goes nowhere', async () => {
  const onRejected = jest.fn();
  await show(<Confirming run={async () => false} onRejected={onRejected} />);
  await fireEvent.press(screen.getByText(c.reject));
  await fireEvent.press(screen.getByText(c.confirmReject));
  expect(onRejected).not.toHaveBeenCalled();
});

test('an unconfirmed claimant is told what they can do, and can leave after a second look', async () => {
  const leave = jest.spyOn(api, 'leaveExchange').mockResolvedValue(undefined);
  await show(<Waiting signed />);

  expect(
    screen.getByText(fill(wording.exchange.waitingConfirmationSigned, 'Ana Ruiz')),
  ).toBeTruthy();
  expect(screen.getByText(c.limits)).toBeTruthy();

  await fireEvent.press(screen.getByText(c.leave));
  expect(leave).not.toHaveBeenCalled();
  expect(screen.getByText(fill(c.leaveText, 'Ana Ruiz'))).toBeTruthy();
  // They signed, so they are told what becomes of the signature.
  expect(screen.getByText(c.leaveVoids)).toBeTruthy();

  await fireEvent.press(screen.getByText(c.confirmLeave));
  await waitFor(() => expect(mockDismissTo).toHaveBeenCalledWith('/'));
  expect(leave).toHaveBeenCalledWith(ID);
});

test('a claimant who has not signed is not told about a signature', async () => {
  await show(<Waiting signed={false} />);
  expect(screen.getByText(fill(wording.exchange.waitingConfirmation, 'Ana Ruiz'))).toBeTruthy();
  await fireEvent.press(screen.getByText(c.leave));
  expect(screen.queryByText(c.leaveVoids)).toBeNull();
});

test('leaving that is refused says why and stays on the exchange', async () => {
  jest.spyOn(api, 'leaveExchange').mockRejectedValue(new ApiFailure('ACTION_NOT_ALLOWED'));
  await show(<Waiting signed={false} />);
  await fireEvent.press(screen.getByText(c.leave));
  await fireEvent.press(screen.getByText(c.confirmLeave));

  await waitFor(() => expect(screen.getByText(wording.errors.ACTION_NOT_ALLOWED)).toBeTruthy());
  expect(mockDismissTo).not.toHaveBeenCalled();
});

function Blocking({ claimant }: { claimant: boolean }) {
  const actions = useFakeActions(async () => true);
  return (
    <ExchangeSafety
      exchange={exchange(
        claimant ? { you: 'B', claimant: null } : { you: 'B', counterparty: 'CONFIRMED' },
      )}
      otherName="Ana Ruiz"
      actions={actions}
      reload={mockReload}
    />
  );
}
const mockReload = jest.fn(async () => null);

test('a block by an unconfirmed claimant says it takes them out, and then does', async () => {
  jest.spyOn(api, 'blockStatus').mockResolvedValue({ blocked: false, name: 'Ana Ruiz' });
  const block = jest.spyOn(api, 'block').mockResolvedValue(undefined);
  await show(<Blocking claimant />);

  await fireEvent.press(await screen.findByText(fill(wording.safety.block, 'Ana Ruiz')));
  expect(screen.getByText(c.blockLeaves)).toBeTruthy();
  // They have nothing to decline and no agreement to keep.
  expect(screen.queryByText(wording.safety.blockEnds)).toBeNull();
  expect(screen.queryByText(wording.safety.blockKeeps)).toBeNull();

  await fireEvent.press(screen.getByText(fill(wording.safety.confirmBlock, 'Ana Ruiz')));
  await waitFor(() => expect(mockDismissTo).toHaveBeenCalledWith('/'));
  expect(block).toHaveBeenCalledWith(ID);
  // The exchange is gone for them: there is nothing to read again.
  expect(mockReload).not.toHaveBeenCalled();
});

test('a block by a party is the block it always was', async () => {
  jest.spyOn(api, 'blockStatus').mockResolvedValue({ blocked: false, name: 'Ana Ruiz' });
  jest.spyOn(api, 'block').mockResolvedValue(undefined);
  await show(<Blocking claimant={false} />);

  await fireEvent.press(await screen.findByText(fill(wording.safety.block, 'Ana Ruiz')));
  expect(screen.getByText(wording.safety.blockEnds)).toBeTruthy();
  expect(screen.queryByText(c.blockLeaves)).toBeNull();

  await fireEvent.press(screen.getByText(fill(wording.safety.confirmBlock, 'Ana Ruiz')));
  await waitFor(() => expect(mockReload).toHaveBeenCalled());
  expect(mockDismissTo).not.toHaveBeenCalled();
});
