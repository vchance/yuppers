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
  proof_required: true,
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
    addIdentifier: async (identifier: string, code: string, proof?: string) => {
      service.calls.push(proof ? `add ${identifier} ${proof}` : `add ${identifier}`);
      const current = identifier.includes('@') ? service.current.email : service.current.phone;
      const any = service.current.email || service.current.phone;
      if (any && current !== identifier && proof !== 'p0') {
        throw new ApiFailure('PROOF_REQUIRED');
      }
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
    proveIdentifier: async (channel: 'EMAIL' | 'PHONE', code: string) => {
      service.calls.push(`prove ${channel}`);
      if (code !== '123456') throw new ApiFailure('INVALID_CODE');
      return { proof: 'p0', expires_at: '2026-10-22T09:10:00Z' };
    },
    combineAccounts: async (token: string, proof?: string) => {
      service.calls.push(`combine ${token}`);
      if (proof !== 'p0') throw new ApiFailure('PROOF_REQUIRED');
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

test('a number is added after a code to the email, with a code once the box is ticked, then removed', async () => {
  const service = stand();
  const { wording } = await show(service);
  const w = wording.identifiers;
  await fireEvent.press(screen.getByTestId('phone-add'));
  expect(await screen.findByRole('header', { name: w.addPhoneTitle })).toBeTruthy();
  // A session alone adds no way in: first a code to the email.
  expect(
    await screen.findByText(w.proveAddIntro.replace('{identifier}', 'ana@example.test')),
  ).toBeTruthy();
  await fireEvent.press(
    screen.getByRole('button', { name: w.proveSend.replace('{identifier}', 'ana@example.test') }),
  );
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.proveConfirm }));
  await fireEvent.changeText(await screen.findByLabelText(w.newPhoneLabel), '856 548 8780');
  const send = () => screen.getByRole('button', { name: w.sendCode });
  expect(send().props.accessibilityState).toMatchObject({ disabled: true });
  await fireEvent.press(screen.getByRole('checkbox'));
  await fireEvent.press(send());
  expect(service.calls).toEqual(['code ana@example.test', 'prove EMAIL', 'code +18565488780']);
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.confirm }));
  expect(await screen.findByText('(856) 548-8780')).toBeTruthy();
  expect(service.calls.at(-1)).toBe('add +18565488780 p0');

  // The number now can go, with a code sent to the email that stays.
  const remove = screen.getByTestId('phone-remove');
  expect(remove.props.accessibilityState).toMatchObject({ disabled: false });
  await fireEvent.press(remove);
  expect(await screen.findByRole('header', { name: w.removePhoneTitle })).toBeTruthy();
  await fireEvent.press(
    screen.getByRole('button', { name: w.sendRemovalCode.replace('{staying}', 'ana@example.test') }),
  );
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.removeConfirm }));
  expect(await screen.findByText(w.removed)).toBeTruthy();
  expect(service.calls.slice(-2)).toEqual(['code ana@example.test', 'remove phone']);
});

test('a number on another account offers to combine, and combines on its button', async () => {
  const service = stand({ otherAccountAt: '+18565488780' });
  const { wording } = await show(service);
  const w = wording.combine;
  await fireEvent.press(screen.getByTestId('phone-add'));
  await fireEvent.press(
    await screen.findByRole('button', {
      name: wording.identifiers.proveSend.replace('{identifier}', 'ana@example.test'),
    }),
  );
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: wording.identifiers.proveConfirm }));
  await fireEvent.changeText(
    await screen.findByLabelText(wording.identifiers.newPhoneLabel),
    '8565488780',
  );
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

test('changing the email takes a code to the account’s own first', async () => {
  const service = stand();
  const { wording } = await show(service);
  const w = wording.identifiers;
  await fireEvent.press(screen.getByTestId('email-change'));
  expect(
    await screen.findByText(w.proveIntro.replace('{identifier}', 'ana@example.test')),
  ).toBeTruthy();
  await fireEvent.press(
    screen.getByRole('button', { name: w.proveSend.replace('{identifier}', 'ana@example.test') }),
  );
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.proveConfirm }));
  expect(
    await screen.findByText(w.changeEmailTold.replace('{identifier}', 'ana@example.test')),
  ).toBeTruthy();
  await fireEvent.changeText(screen.getByLabelText(w.newEmailLabel), 'new@example.test');
  await fireEvent.press(screen.getByRole('button', { name: w.sendCode }));
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: w.confirm }));
  expect(await screen.findByText('new@example.test')).toBeTruthy();
  expect(service.calls).toEqual([
    'code ana@example.test',
    'prove EMAIL',
    'code new@example.test',
    'add new@example.test p0',
  ]);
});

test('a notice with no email to tell is shown once, until dismissed', async () => {
  const wording = wordingFor('en');
  const updated = jest.spyOn(api, 'updateMe').mockResolvedValue({ ...account, notice: null });
  const setAccount = jest.fn();
  const session = {
    account: { ...account, notice: { kind: 'PHONE_REMOVED', at: '2026-10-22T09:00:00Z' } },
    setAccount,
  } as unknown as Session;
  await render(wrap(<CombinedNotice />, session));
  expect(
    await screen.findByText(/^The phone number on your Yuppers account was removed on/),
  ).toBeTruthy();
  await fireEvent.press(screen.getByRole('button', { name: wording.combine.noticeDismiss }));
  expect(updated).toHaveBeenCalledWith({ dismiss_notice: true });
  expect(setAccount).toHaveBeenCalledWith({ ...account, notice: null });
  updated.mockRestore();
});

test('an invitation sent to another address takes a proof, then the address typed, and opens', async () => {
  const wording = wordingFor('en');
  const w = wording.invitation;
  const sentTo: BoundAddress = { kind: 'EMAIL', replaces: true };
  const calls: string[] = [];
  const client: InvitationAddressApi = {
    requestCode: async (identifier: string) => {
      calls.push(`code ${identifier}`);
    },
    proveIdentifier: async (channel: 'EMAIL' | 'PHONE') => {
      calls.push(`prove ${channel}`);
      return { proof: 'p0', expires_at: '2026-10-22T09:10:00Z' };
    },
    requestInvitationAddressCode: async (token: string, identifier: string) => {
      calls.push(`address ${token} ${identifier}`);
      if (identifier !== 'j@gmail.com') throw new ApiFailure('NOT_INVITED_ADDRESS');
    },
    addInvitationAddress: async (
      _token: string,
      identifier: string,
      code: string,
      replace: boolean,
      proof?: string,
    ) => {
      calls.push(`add ${identifier} ${code} ${replace} ${proof}`);
      return { id: 'x' } as ExchangeView;
    },
    combineAccounts: async () => account,
    claimInvitation: async () => ({ id: 'x' }) as ExchangeView,
  };
  const opened = jest.fn();
  const session = { account, setAccount: jest.fn() } as unknown as Session;
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
  expect(await screen.findByRole('header', { name: w.sentToEmail })).toBeTruthy();
  expect(screen.getByText(w.sentToReplacesEmail)).toBeTruthy();
  // An address needs no box.
  expect(screen.queryByRole('checkbox')).toBeNull();
  await fireEvent.press(screen.getByTestId('send-proof-code'));
  await fireEvent.changeText(screen.getByLabelText(wording.signIn.codeLabel), '123456');
  await fireEvent.press(screen.getByRole('button', { name: wording.identifiers.proveConfirm }));
  await fireEvent.changeText(
    await screen.findByLabelText(wording.identifiers.newEmailLabel),
    'other@gmail.com',
  );
  await fireEvent.press(screen.getByTestId('send-address-code'));
  expect(await screen.findByText(wording.errors.NOT_INVITED_ADDRESS)).toBeTruthy();
  await fireEvent.changeText(screen.getByLabelText(wording.identifiers.newEmailLabel), 'j@gmail.com');
  await fireEvent.press(screen.getByTestId('send-address-code'));
  await fireEvent.changeText(
    await screen.findByLabelText(wording.signIn.codeLabel),
    '123456',
  );
  await fireEvent.press(screen.getByRole('button', { name: w.replaceAndOpen }));
  expect(calls).toEqual([
    'code ana@example.test',
    'prove EMAIL',
    'address t0 other@gmail.com',
    'address t0 j@gmail.com',
    'add j@gmail.com 123456 true p0',
  ]);
  expect(opened).toHaveBeenCalledWith({ id: 'x' });
});
