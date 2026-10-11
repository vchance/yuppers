import { TERMS_VERSION } from '@yuppers/shared';
import * as SecureStore from 'expo-secure-store';

import { tokenStore } from '../token-store';

// The client takes hold of `fetch` when it is made, so the stand-in for the
// service has to be in place before the session module is loaded.
let service: typeof fetch = async () => {
  throw new TypeError('Network request failed');
};
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service(...args)) as typeof fetch;
const { api, dropSession, keepSession, restoreSession } =
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  require('../session') as typeof import('../session');

jest.mock('expo-secure-store', () => ({
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: 'when-unlocked-this-device-only',
  getItemAsync: jest.fn(async () => null),
  setItemAsync: jest.fn(async () => {}),
  deleteItemAsync: jest.fn(async () => {}),
}));

jest.mock('expo-crypto', () => {
  let next = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, '0')}` };
});

const secure = jest.mocked(SecureStore);
const deviceOnly = { keychainAccessible: 'when-unlocked-this-device-only' };

/** What the app sent, as the service would see it. */
interface Sent {
  url: string;
  method: string;
  authorization: string | null;
  idempotencyKey: string | null;
  body: unknown;
}

function serviceAnswering(body: unknown, status = 200) {
  const sent: Sent[] = [];
  service = jest.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = input instanceof Request ? input : new Request(input, init);
    const text = await request.text();
    sent.push({
      url: request.url,
      method: request.method,
      authorization: request.headers.get('Authorization'),
      idempotencyKey: request.headers.get('Idempotency-Key'),
      body: text ? JSON.parse(text) : null,
    });
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
  return sent;
}

beforeEach(async () => {
  jest.clearAllMocks();
  await dropSession();
  jest.clearAllMocks();
});

test('on a device the token store is the secure storage, and only that', async () => {
  await tokenStore.write('s3cret');
  expect(secure.setItemAsync).toHaveBeenCalledWith('yuppers.session', 's3cret', deviceOnly);

  secure.getItemAsync.mockResolvedValueOnce('s3cret');
  expect(await tokenStore.read()).toBe('s3cret');
  expect(secure.getItemAsync).toHaveBeenCalledWith('yuppers.session', deviceOnly);

  await tokenStore.clear();
  expect(secure.deleteItemAsync).toHaveBeenCalledWith('yuppers.session', deviceOnly);
});

test('the browser stand-in refuses to load on a device', () => {
  // The bundler never offers it to iOS or Android; this is the second lock.
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  expect(() => require('../token-store.web')).toThrow(/outside the web target/);
});

test('signing in asks for a token, and the token kept goes out as a bearer token', async () => {
  const account = { id: 'a', display_name: 'Ana', adult_confirmed: true, notification_detail: false, language: 'en' };
  let sent = serviceAnswering({ account, token: 'issued-token' });
  const created = await api.signIn('ana@example.test', '123456', 'es');
  expect(sent[0].url).toMatch(/\/v1\/auth\/sessions$/);
  expect(sent[0].body).toEqual({
    identifier: 'ana@example.test',
    code: '123456',
    delivery: 'TOKEN',
    language: 'es',
    terms_version: TERMS_VERSION,
  });

  await keepSession(created.token ?? '');
  expect(secure.setItemAsync).toHaveBeenCalledWith('yuppers.session', 'issued-token', deviceOnly);

  sent = serviceAnswering([]);
  await api.listExchanges();
  expect(sent[0].authorization).toBe('Bearer issued-token');
});

test('a session left by an earlier launch is picked up from secure storage', async () => {
  secure.getItemAsync.mockResolvedValueOnce('from-last-time');
  await restoreSession();
  const sent = serviceAnswering({ id: 'a' });
  await api.me();
  expect(sent[0].authorization).toBe('Bearer from-last-time');
});

test('with nothing stored, or storage that cannot be read, nobody is signed in', async () => {
  const sent = serviceAnswering({});
  await restoreSession();
  expect(await api.me()).toBeNull();

  secure.getItemAsync.mockRejectedValueOnce(new Error('keychain unavailable'));
  await restoreSession();
  expect(await api.me()).toBeNull();
  expect(sent).toEqual([]);
});

test('signing out forgets the token here and in secure storage', async () => {
  await keepSession('issued-token');
  await dropSession();
  expect(secure.deleteItemAsync).toHaveBeenCalledWith('yuppers.session', deviceOnly);
  const sent = serviceAnswering({});
  expect(await api.me()).toBeNull();
  expect(sent).toEqual([]);
});

test('a change names its version and carries an unguessable key of its own', async () => {
  await keepSession('issued-token');
  const sent = serviceAnswering({ id: 'x', version: 5 });
  const id = '0b9f1c2e-7a41-4c6e-9a55-3d2f8e1b6c70';
  await api.runCommand(id, 4, { type: 'PROPOSE_END' });
  await api.runCommand(id, 5, { type: 'CANCEL_END' });

  expect(sent[0].url).toMatch(new RegExp(`/v1/exchanges/${id}/commands$`));
  expect(sent[0].body).toEqual({ expected_version: 4, command: { type: 'PROPOSE_END' } });
  expect(sent[0].idempotencyKey).toMatch(/^[0-9a-f-]{36}$/);
  expect(sent[1].idempotencyKey).toMatch(/^[0-9a-f-]{36}$/);
  expect(sent[1].idempotencyKey).not.toBe(sent[0].idempotencyKey);
});

test('an invitation token goes to the service in the body, never in the address', async () => {
  const invitation = 'a3'.repeat(32);
  const sent = serviceAnswering({});
  await api.previewInvitation(invitation);
  await api.claimInvitation(invitation);
  for (const request of sent) {
    expect(request.url).not.toContain(invitation);
    expect(request.body).toEqual({ token: invitation });
  }
});
