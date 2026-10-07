/*
 * What the SMS provider's registration and the US carriers ask the privacy
 * policy and the terms to say, checked on a page as a reader gets it: the
 * app's own page (`screens/LegalPage.test.tsx`) and the static page the
 * build writes (`build/legal-pages.test.ts`), in every language. It returns
 * what is missing, one line each, so a test lists all of it at once.
 *
 * The words are the owner's, given for the registration; a document may say
 * more around them, never less.
 */

type Document = 'privacy' | 'terms'

interface Required {
  /** In the opening paragraph: the site by name. */
  site: string
  /** The privacy policy's section on what is collected, by its heading. */
  collect: string
  /** Not sold, not shared for marketing: in the body and in the section on texts. */
  notSold: string
  mobile: string
  rates: string
  carriers: string
  /**
   * In both documents' sections on texts: the program by name, with its
   * frequency, and the one-time codes apart, said to be Twilio Verify's,
   * with theirs.
   */
  programs: string[]
  frequencies: Record<Document, string[]>
  /** Said of codes by text: that Twilio Verify sends them, and their heading in the terms. */
  codes: { verify: string; heading: string }
  /** The name codes went by as a program of ours, which no document may use any more. */
  retired: string
  /** The opt-in wording of agreement updates, quoted on the terms. */
  consent: string
}

const REQUIRED: Record<string, Required> = {
  en: {
    site: 'Yuppers.app (https://yuppers.app)',
    collect: 'What we collect and how we use it',
    notSold:
      'We do not sell your personal information. We do not share your personal information or your SMS opt-in data and consent with third parties or affiliates for marketing or promotional purposes.',
    mobile:
      'Mobile information will not be shared with third parties or affiliates for marketing or promotional purposes.',
    rates: 'Message and data rates may apply.',
    carriers: 'Carriers are not liable for delayed or undelivered messages.',
    programs: ['Yuppers.app agreement updates'],
    frequencies: {
      terms: [
        'Message frequency: One message per code request.',
        'Message frequency: Message frequency varies; there is no fixed maximum. One text per status change of an agreement you turned updates on for.',
      ],
      privacy: [
        'Message frequency: one message for each code you ask for.',
        'Message frequency varies; there is no fixed maximum: one text per status change of an agreement you turned updates on for.',
      ],
    },
    consent:
      '“Receive text updates from yuppers.app about this agreement, one text per status change. Message frequency varies; there is no fixed maximum. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.”',
    codes: { verify: 'Twilio Verify', heading: 'One-time codes by text' },
    retired: 'Yuppers.app sign-in codes',
  },
  es: {
    site: 'Yuppers.app (https://yuppers.app)',
    collect: 'Qué recopilamos y cómo lo usamos',
    notSold:
      'No vendemos tu información personal. No compartimos tu información personal ni tus datos y consentimiento de suscripción a SMS con terceros ni con afiliados con fines de marketing o promoción.',
    mobile:
      'La información móvil no se compartirá con terceros ni con afiliados con fines de marketing o promoción.',
    // The carriers' phrase, in English too, beside the Spanish.
    rates: 'Message and data rates may apply.',
    carriers: 'Los operadores no son responsables de los mensajes retrasados o no entregados.',
    programs: ['Yuppers.app agreement updates'],
    frequencies: {
      terms: [
        'Frecuencia de los mensajes: un mensaje por cada código que pides.',
        'Frecuencia de los mensajes: la frecuencia de los mensajes varía; no hay un máximo fijo. Un mensaje por cada cambio de estado de un acuerdo para el que activaste las actualizaciones.',
      ],
      privacy: [
        'Frecuencia de los mensajes: un mensaje por cada código que pides.',
        'La frecuencia de los mensajes varía; no hay un máximo fijo: un mensaje por cada cambio de estado de un acuerdo para el que activaste las actualizaciones.',
      ],
    },
    consent:
      '“Recibir actualizaciones por mensaje de texto de yuppers.app sobre este acuerdo, un mensaje por cada cambio de estado. La frecuencia de los mensajes varía; no hay un máximo fijo. Pueden aplicarse tarifas por mensajes y datos. Responde HELP para obtener ayuda o STOP para cancelar. Términos: https://yuppers.app/terms. Política de privacidad: https://yuppers.app/privacy.”',
    codes: { verify: 'Twilio Verify', heading: 'Códigos de un solo uso por mensaje de texto' },
    retired: 'Yuppers.app sign-in codes',
  },
}

const SUPPORT = 'mailto:support@yuppers.app'

/** What `page`, `document` in `language`, lacks of what the registration asks for. */
export function smsRequirements(page: ParentNode, document: Document, language: string): string[] {
  const required = REQUIRED[language]
  if (!required) return [`no requirements written for ${language}`]
  const problems: string[] = []
  const section = (id: string) => page.querySelector(`#${id}`)?.closest('section') ?? null
  const need = (where: Element | null, what: string, label: string) => {
    if (!where) problems.push(`${label}: the section is missing`)
    else if (!where.textContent?.includes(what)) problems.push(`${label}: lacks “${what}”`)
  }

  const opening = page.querySelector('main section p')
  need(opening, required.site, 'the opening paragraph')

  const texts = section('text-messages')
  for (const program of required.programs) need(texts, program, '#text-messages, the program')
  for (const frequency of required.frequencies[document]) need(texts, frequency, '#text-messages, the frequency')
  need(texts, required.rates, '#text-messages')
  need(texts, required.carriers, '#text-messages')
  need(texts, required.notSold, '#text-messages')
  need(texts, required.mobile, '#text-messages')
  // One-time codes, apart from the program, and Twilio Verify's.
  need(texts, required.codes.verify, '#text-messages, the one-time codes')
  if (texts?.textContent?.includes(required.retired)) {
    problems.push(`#text-messages: still names “${required.retired}” as a program`)
  }
  // HELP and STOP, each inside a <strong>.
  const strong = [...(texts?.querySelectorAll('strong') ?? [])].map((element) => element.textContent ?? '')
  for (const keyword of ['HELP', 'STOP']) {
    if (!strong.some((text) => text.includes(keyword))) {
      problems.push(`#text-messages: ${keyword} is not inside a <strong>`)
    }
  }
  if (!texts?.querySelector(`a[href="${SUPPORT}"]`)) {
    problems.push('#text-messages: no mailto: link to support')
  }

  if (document === 'privacy') {
    const collect = section('what-we-collect')
    if (page.querySelector('#what-we-collect')?.textContent !== required.collect) {
      problems.push(`#what-we-collect: the heading is not “${required.collect}”`)
    }
    need(collect, required.notSold, '#what-we-collect')
  } else {
    need(texts, required.consent, '#text-messages, the opt-in wording')
    // The codes under a heading of their own.
    const headings = [...(texts?.querySelectorAll('h3') ?? [])].map((h) => h.textContent ?? '')
    if (!headings.includes(required.codes.heading)) {
      problems.push(`#text-messages: no heading “${required.codes.heading}”`)
    }
    // A real link to the privacy policy's section on texts.
    const link = [...(texts?.querySelectorAll('a') ?? [])].find((a) =>
      /^\/([a-z]{2}\/)?privacy#text-messages$/.test(a.getAttribute('href') ?? ''),
    )
    if (!link) problems.push('#text-messages: no link to /privacy#text-messages')
  }
  return problems
}
