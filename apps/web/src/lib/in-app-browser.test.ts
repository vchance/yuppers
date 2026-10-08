import { describe, expect, test } from 'vitest'

import { addressToOpen, detectInAppBrowser, openElsewhereLink } from './in-app-browser'

/*
 * User agents as the apps and browsers send them, collected from published
 * captures. Each app's token is what matters; the versions around it change
 * with every release. The Outlook one is the system web view's user agent
 * with the token Outlook sends from its own requests (`Outlook-iOS/…`): that
 * its link browser adds it has not been confirmed, which is why the
 * unnamed iOS web view below, the same without the token, is the case that
 * catches it if it does not.
 */
const IOS_WEBVIEW =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148'

const IN_APP: [string, string | null, 'ios' | 'android', number | null, string][] = [
  ['Outlook, iOS', 'Outlook', 'ios', 18, `${IOS_WEBVIEW} Outlook-iOS/749.4.prod.iphone (4.2534.0)`],
  ['an app’s own web view, iOS (unnamed)', null, 'ios', 18, IOS_WEBVIEW],
  [
    'an app’s own web view, iPad (unnamed)',
    null,
    'ios',
    17,
    'Mozilla/5.0 (iPad; CPU OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148',
  ],
  [
    'the Google app (GSA), iOS',
    'Google',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) GSA/322.0.648915268 Mobile/15E148 Safari/604.1',
  ],
  [
    'Facebook, iOS',
    'Facebook',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 [FBAN/FBIOS;FBAV/473.0.0.34.108;FBBV/635414428;FBDV/iPhone15,2;FBMD/iPhone;FBSN/iOS;FBSV/17.5.1;FBSS/3;FBID/phone;FBLC/en_US;FBOP/5;FBRV/0]',
  ],
  [
    'Facebook, Android',
    'Facebook',
    'android',
    null,
    'Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/AP2A.240805.005; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/127.0.6533.103 Mobile Safari/537.36 [FB_IAB/FB4A;FBAV/478.0.0.43.115;]',
  ],
  [
    'Messenger, iOS',
    'Messenger',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_2_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 LightSpeed [FBAN/MessengerLiteForiOS;FBAV/445.0.0.33.112;FBBV/551297367;FBDV/iPhone14,7;FBMD/iPhone;FBSN/iOS;FBSV/17.2.1;FBSS/3;FBCR/;FBID/phone;FBLC/en_US;FBOP/0]',
  ],
  [
    'Instagram, iOS',
    'Instagram',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Instagram 330.0.3.24.105 (iPhone14,5; iOS 17_4; en_US; en; scale=3.00; 1170x2532; 595393270)',
  ],
  [
    'Instagram, Android',
    'Instagram',
    'android',
    null,
    'Mozilla/5.0 (Linux; Android 13; SM-S918B Build/TP1A.220624.014; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/120.0.6099.230 Mobile Safari/537.36 Instagram 315.0.0.29.109 Android (33/13; 480dpi; 1080x2340; samsung; SM-S918B; dm3q; qcom; en_US; 556277207)',
  ],
  [
    'LinkedIn, iOS 16',
    'LinkedIn',
    'ios',
    16,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 [LinkedInApp]/9.29.6650',
  ],
  [
    'Snapchat, iOS (says “like Safari/”)',
    'Snapchat',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_1_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Snapchat/12.65.0.36 (like Safari/8617.2.4.10.8, panda)',
  ],
  [
    'TikTok, iOS',
    'TikTok',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 musical_ly_32.5.0 JsSdk/2.0 NetType/WIFI Channel/App Store ByteLocale/en Region/US ByteFullLocale/en isDarkMode/0 WKWebView/1 RevealType/Dialog BytedanceWebview/d8a21c6',
  ],
  [
    'TikTok, Android',
    'TikTok',
    'android',
    null,
    'Mozilla/5.0 (Linux; Android 12; SM-A525F Build/SP1A.210812.016; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/119.0.6045.163 Mobile Safari/537.36 trill_320405 JsSdk/1.0 NetType/WIFI Channel/googleplay AppName/musical_ly app_version/32.4.5 ByteLocale/en Region/GB BytedanceWebview/d8a21c6',
  ],
  [
    'X, iOS',
    'X',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Twitter for iPhone/10.10',
  ],
  [
    'X, Android',
    'X',
    'android',
    null,
    'Mozilla/5.0 (Linux; Android 13; Pixel 7 Build/TQ3A.230805.001; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/116.0.0.0 Mobile Safari/537.36 TwitterAndroid',
  ],
  [
    'LINE, iOS (says “Safari” without a version)',
    'LINE',
    'ios',
    17,
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari Line/14.1.0',
  ],
  [
    'an app’s own web view, Android (unnamed)',
    null,
    'android',
    null,
    'Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/AP2A.240805.005; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/128.0.6613.88 Mobile Safari/537.36',
  ],
]

const BROWSERS: [string, string][] = [
  [
    'Safari, iPhone (and SFSafariViewController, which sends the same)',
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
  ],
  [
    'Safari, iPad',
    'Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  ],
  [
    'Safari, iPad asking for the desktop site',
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15',
  ],
  [
    'Chrome, iOS',
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/127.0.6533.107 Mobile/15E148 Safari/604.1',
  ],
  [
    'Firefox, iOS',
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/128.0 Mobile/15E148 Safari/605.1.15',
  ],
  [
    'Edge, iOS',
    'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 EdgiOS/127.0.2651.102 Mobile/15E148 Safari/604.1',
  ],
  [
    'Chrome, Android',
    'Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Mobile Safari/537.36',
  ],
  [
    'Samsung Internet, Android',
    'Mozilla/5.0 (Linux; Android 14; SAMSUNG SM-S921B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36',
  ],
  ['Firefox, Android', 'Mozilla/5.0 (Android 14; Mobile; rv:129.0) Gecko/129.0 Firefox/129.0'],
  [
    'Chrome, Mac',
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36',
  ],
  [
    'Edge, Windows',
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36 Edg/127.0.2651.86',
  ],
]

describe('an in-app browser', () => {
  test.each(IN_APP)('is told: %s', (_, app, platform, iosVersion, userAgent) => {
    expect(detectInAppBrowser(userAgent)).toEqual({ app, platform, iosVersion })
  })

  test.each(BROWSERS)('a browser proper is not: %s', (_, userAgent) => {
    expect(detectInAppBrowser(userAgent)).toBeNull()
  })

  test('a page added to the home screen is not one, though it sends no Safari token', () => {
    expect(detectInAppBrowser(IOS_WEBVIEW, true)).toBeNull()
  })
})

describe('opening the page elsewhere', () => {
  const at = (platform: 'ios' | 'android', iosVersion: number | null) => ({
    app: null,
    platform,
    iosVersion,
  })

  test('Safari’s own scheme on iOS 17 and later, the app’s menu before', () => {
    expect(openElsewhereLink(at('ios', 17), 'https://yuppers.app/')).toBe(
      'x-safari-https://yuppers.app/',
    )
    expect(openElsewhereLink(at('ios', 18), 'https://yuppers.app/es/i#abcdefghijklmnop')).toBe(
      'x-safari-https://yuppers.app/es/i#abcdefghijklmnop',
    )
    expect(openElsewhereLink(at('ios', 16), 'https://yuppers.app/')).toBeNull()
    expect(openElsewhereLink(at('ios', null), 'https://yuppers.app/')).toBeNull()
  })

  test('an intent for the default browser on Android, the fragment before its own', () => {
    expect(openElsewhereLink(at('android', null), 'https://yuppers.app/account')).toBe(
      'intent://yuppers.app/account#Intent;scheme=https;end',
    )
    expect(openElsewhereLink(at('android', null), 'https://yuppers.app/en/i#abcdefghijklmnop')).toBe(
      'intent://yuppers.app/en/i#abcdefghijklmnop#Intent;scheme=https;end',
    )
  })

  test('plain HTTP, in development, the same way; nothing anywhere else', () => {
    expect(openElsewhereLink(at('android', null), 'http://localhost:5173/')).toBe(
      'intent://localhost:5173/#Intent;scheme=http;end',
    )
    expect(openElsewhereLink(at('ios', 17), 'http://127.0.0.1:8090/')).toBe(
      'x-safari-http://127.0.0.1:8090/',
    )
    expect(
      openElsewhereLink({ app: 'Facebook', platform: 'other', iosVersion: null }, 'https://yuppers.app/'),
    ).toBeNull()
  })

  test('the page’s address without its query, and an invitation’s token only on its page', () => {
    const location = {
      origin: 'https://yuppers.app',
      pathname: '/exchanges/1',
      search: '?code=123456',
      hash: '#x',
    } as Location
    expect(addressToOpen(location, null)).toBe('https://yuppers.app/exchanges/1')
    expect(addressToOpen({ ...location, pathname: '/es/i' }, 'abcdefghijklmnop')).toBe(
      'https://yuppers.app/es/i#abcdefghijklmnop',
    )
  })
})
