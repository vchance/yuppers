import type { Account, ExchangeView } from '@yuppers/api-client';
import {
  ApiFailure,
  createI18n,
  wordingFor,
  type BoundAddress,
  type CombineOffer as Offer,
  type IdentifiersApi,
  type InvitationAddressApi,
} from '@yuppers/shared';
import { fireEvent, render, screen } from '@testing-library/react-native';
import { useMemo, useState } from 'react';

import { I18nContext, SessionContext, type Session } from '../../lib/context';
import { api } from '../../lib/session';
import { CombinedNotice } from '../CombinedNotice';
import { Identifiers } from '../Identifiers';
import { InvitationAddress } from '../InvitationAddress';

/*
 * The account's email address and phone number (README, "Combining
 * accounts"), and an invitation sent to another address, on the app.
 */

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));

const account: Account = {
  id: 'a0000000-0000-4000-8000-000000000001',
  display_name: 'Ana Ruiz',
  adult_confirmed: true,
  language: 'en',
  email: 'ana@example.test',
};

const OFFER: Offer = {
  token: 'd6'.repeat(32),
  expires_at: '2026-10-22T09:10:00Z',
  proved: 'PHONE',
  other: {
    display_name: 'Ana R.',
    email: null,
    phone: '(•••) •••-8780',
    yups: { drafts: 0, negotiating: 0, in_force: 1, closed: 0 },
    payment_options: false,
    text_updates: true,
    devices: true,
  },
  email: 'KEPT',
  phone: 'ADDED',
  payment_options_move: false,
  text_updates_end: false,
};

interface Stand extends IdentifiersApi {
  current: Account;
  calls: string[];
  otherAccountAt: string | null;
}

function stand(prepare: Partial<Stand> = {}): Stand {
  const service: Stand = {
    current: account,
    calls: [],
    otherAccountAt: null,
    ...prepare,
    requestCode: async (identifier: string) => {
      service.calls.push(`code ${identifier}`);
    },
    addIdentifier: async (identifier: string, code: string) => {
      service.calls.push(`add ${identifier}`);
      if (code !== '123456') throw new ApiFailure('INVALID_CODE');
      if (identifier === service.otherAccountAt) {
        throw new ApiFailure('IDENTIFIER_ON_OTHER_ACCOUNT', false, OFFER);
      }
      service.current = identifier.includes('@')
        ? { ...service.current, email: identifier }
        : { ...service.current, phone: identifier };
      return service.current;
    },
    removeIdentifier: async (kind: 'email' | 'phone', code: string) => {
      service.calls.push(`remove ${kind}`);
      if (code !== '123456') throw new ApiFailure('INVALID_CODE');
      service.current = { ...service.current, [kind]: null };
      return service.current;
    },
    combineAccounts: async (token: string) => {
      service.calls.push(`combine ${token}`);
      service.current = { ...service.current, phone: '+18565488780' };
      return service.current;
    },
  };
  return service;
}

function wrap(node: React.ReactNode, session: Session) {
  const wording = wordingFor('en');
  return (
    <I18nContext.Provider value={createI18n('en', wording, () => {})}>
      <SessionContext.Provider value={session}>{node}</SessionContext.Provider>
    </I18nContext.Provider>
  );
}

/** The section, following the account as the app's session does. */
function Host({ service }: { service: Stand }) {
  const [current, setCurrent] = useState(service.current);
  const session = useMemo(() => ({ setAccount: setCurrent }) as unknown as Session, []);
  return wrap(<Identifiers account={current} client={service} />, session);
}

async function show(service: Stand) {
  await render(<Host service={service} />);
  return { wording: wordingFor('en') };
}

test('Remove waits while there is only one, and says why', async () => {
  const { wording } = await show(stand());
  const w = wording.identifiers;
  expect(await screen.findByRole('header', { name: w.heading })).toBeTruthy();
  const remove = screen.getByTestId('email-remove');
  expect(remove.props.accessibilityState).toMatchObject({ disabled: true });
  expect(remove.props.accessibilityHint).toBe(w.onlyEmail);
  expect(screen.getByText(w.onlyEmail)).toBeTruthy();
  expect(screen.getByText(w.none)).toBeTruthy();
});

test('a number is added with a code once the box is ticked, then the email removed', async () => {
  const service = stand();
  const { wording } = await show(service);
  const w = wording.identifiers;
  await fireEvent.press(screen.getByTestId('phone-add'));
  expect(await screen.findByRole('header', { name: w.addPhoneTitle })).toBeTruthy();
  await fireEvent.changeText(screen.getByLabelText(w.newPhoneLabel), '856 548 8780');
  const send = () => screen.getByRole('button', { name: w.sendCode });
  expect(send().props.accessibilityState).toMatchObject({ disabled: true });
  await fireEvent.press(screen.getByRole('checkbox'));
  await fireEvent.press(send());
  expect(service.calls).toEqual(['code +18565488780']);
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.confirm }));
  expect(await screen.findByText('(856) 548-8780')).toBeTruthy();

  // The email now can go, with a code sent to the number that stays.
  const remove = screen.getByTestId('email-remove');
  expect(remove.props.accessibilityState).toMatchObject({ disabled: false });
  await fireEvent.press(remove);
  expect(await screen.findByRole('header', { name: w.removeEmailTitle })).toBeTruthy();
  await fireEvent.press(screen.getByRole('checkbox'));
  await fireEvent.press(
    screen.getByRole('button', { name: w.sendRemovalCode.replace('{staying}', '(856) 548-8780') }),
  );
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.removeConfirm }));
  expect(await screen.findByText(w.removed)).toBeTruthy();
  expect(service.calls.slice(-2)).toEqual(['code +18565488780', 'remove email']);
});

test('a number on another account offers to combine, and combines on its button', async () => {
  const service = stand({ otherAccountAt: '+18565488780' });
  const { wording } = await show(service);
  const w = wording.combine;
  await fireEvent.press(screen.getByTestId('phone-add'));
  await fireEvent.changeText(screen.getByLabelText(wording.identifiers.newPhoneLabel), '8565488780');
  await fireEvent.press(screen.getByRole('checkbox'));
  await fireEvent.press(screen.getByRole('button', { name: wording.identifiers.sendCode }));
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: wording.identifiers.confirm }));
  expect(await screen.findByRole('header', { name: w.headingPhone })).toBeTruthy();
  for (const line of [
    '• Name: Ana R.',
    '• Phone: (•••) •••-8780',
    `• ${w.movesYups}`,
    '• Its phone number, (•••) •••-8780, is added to this account.',
    `• ${w.textUpdatesMove}`,
    `• ${w.devices}`,
    `• ${w.ends}`,
  ]) {
    expect(screen.getByText(line)).toBeTruthy();
  }
  expect(service.calls).not.toContain(`combine ${OFFER.token}`);
  await fireEvent.press(screen.getByRole('button', { name: w.confirm }));
  expect(await screen.findByText(w.done)).toBeTruthy();
  expect(service.calls).toContain(`combine ${OFFER.token}`);
});

test('accounts combined with no email to tell are told once, until dismissed', async () => {
  const wording = wordingFor('en');
  const updated = jest
    .spyOn(api, 'updateMe')
    .mockResolvedValue({ ...account, combined_notice: null });
  const setAccount = jest.fn();
  const session = {
    account: { ...account, combined_notice: '2026-10-22T09:00:00Z' },
    setAccount,
  } as unknown as Session;
  await render(wrap(<CombinedNotice />, session));
  expect(
    await screen.findByText(/^Two of your Yuppers accounts were combined into this one on/),
  ).toBeTruthy();
  await fireEvent.press(screen.getByRole('button', { name: wording.combine.noticeDismiss }));
  expect(updated).toHaveBeenCalledWith({ dismiss_combined_notice: true });
  expect(setAccount).toHaveBeenCalledWith({ ...account, combined_notice: null });
  updated.mockRestore();
});

test('an invitation sent to another address is added with a code and opens', async () => {
  const wording = wordingFor('en');
  const w = wording.invitation;
  const sentTo: BoundAddress = { kind: 'EMAIL', masked: 'j•••@gmail.com', replaces: true };
  const calls: string[] = [];
  const client: InvitationAddressApi = {
    requestInvitationAddressCode: async (token: string) => {
      calls.push(`code ${token}`);
    },
    addInvitationAddress: async (_token: string, code: string, replace: boolean) => {
      calls.push(`add ${code} ${replace}`);
      return { id: 'x' } as ExchangeView;
    },
    combineAccounts: async () => account,
    claimInvitation: async () => ({ id: 'x' }) as ExchangeView,
  };
  const opened = jest.fn();
  const session = { setAccount: jest.fn() } as unknown as Session;
  await render(
    wrap(
      <InvitationAddress
        token="t0"
        sentTo={sentTo}
        onOpened={opened}
        onSignOut={() => {}}
        client={client}
      />,
      session,
    ),
  );
  expect(
    await screen.findByRole('header', {
      name: w.sentTo.replace('{identifier}', 'j•••@gmail.com'),
    }),
  ).toBeTruthy();
  expect(screen.getByText(w.sentToReplacesEmail)).toBeTruthy();
  // An address needs no box.
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.press(screen.getByTestId('send-address-code'));
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.replaceAndOpen }));
  expect(calls).toEqual(['code t0', 'add 123456 true']);
  expect(opened).toHaveBeenCalledWith({ id: 'x' });
});
