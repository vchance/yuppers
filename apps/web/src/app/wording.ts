import {
  languages,
  pickLanguage,
  legalPathOf,
  type HelpWording,
  type Language,
  type LegalDocument,
  type LegalWording,
  type Wording,
} from '@yuppers/shared'

/*
 * Each language's wording is its own file, fetched when that language is
 * shown. A visitor loads one language, however many the product supports,
 * which keeps the invitation page inside its budget (DESIGN.md §13.5). The
 * glob picks up every file in the shared wording folder, so a new language
 * needs nothing here.
 */
const files = import.meta.glob<Wording>(
  ['../../../../packages/shared/wording/*.json', '!**/languages.json'],
  { import: 'default' },
)

export function loadWording(language: Language): Promise<Wording> {
  const path = Object.keys(files).find((file) => file.endsWith(`/${language}.json`))
  if (!path) return Promise.reject(new Error(`no wording file for ${language}`))
  return files[path]()
}

/*
 * The help pages' text is a file of its own per language, fetched only by
 * the help pages, so that the wording every other page loads first does not
 * carry it.
 */
const helpFiles = import.meta.glob<HelpWording>('../../../../packages/shared/wording/help/*.json', {
  import: 'default',
})

export function loadHelp(language: Language): Promise<HelpWording> {
  const path = Object.keys(helpFiles).find((file) => file.endsWith(`/${language}.json`))
  if (!path) return Promise.reject(new Error(`no help file for ${language}`))
  return helpFiles[path]()
}

/*
 * The privacy policy's and the terms' text, likewise a file of its own per
 * document and language, fetched only by their page. Each is kept once it
 * has arrived, so that a page that fetched it before the app first drew
 * (`main.tsx`) shows it at once.
 */
const legalFiles: Record<LegalDocument, Record<string, () => Promise<LegalWording>>> = {
  privacy: import.meta.glob<LegalWording>('../../../../packages/shared/wording/privacy/*.json', {
    import: 'default',
  }),
  terms: import.meta.glob<LegalWording>('../../../../packages/shared/wording/terms/*.json', {
    import: 'default',
  }),
}
const legalLoaded = new Map<string, LegalWording>()

export function loadLegal(document: LegalDocument, language: Language): Promise<LegalWording> {
  const files = legalFiles[document]
  const path = Object.keys(files).find((file) => file.endsWith(`/${language}.json`))
  if (!path) return Promise.reject(new Error(`no ${document} document in ${language}`))
  return files[path]().then((wording) => {
    legalLoaded.set(`${document}/${language}`, wording)
    return wording
  })
}

/** The document in `language`, if it has already arrived. */
export function loadedLegal(document: LegalDocument, language: Language): LegalWording | null {
  return legalLoaded.get(`${document}/${language}`) ?? null
}

/**
 * The language the address asks for, if it is one we have: with `?lang=`,
 * as the mobile app opens help pages, in a browser that does not know which
 * language the app is in; or as the privacy policy's or the terms' address
 * names it, `/{language}/privacy`. It is not remembered.
 */
export function addressLanguage(): Language | null {
  const asked = new URLSearchParams(window.location.search).get('lang')?.toLowerCase()
  const found = languages.find((info) => info.code.toLowerCase() === asked)?.code
  if (found) return found
  // `/privacy` names no language: it is where anyone lands who named none.
  const legal = legalPathOf(window.location.pathname)
  if (!legal?.named) return null
  return languages.find((info) => info.code === legal.language)?.code ?? null
}

const CHOICE = 'yuppers.language'

/**
 * The language to show before anyone has signed in: the one chosen in the
 * picker on this device, otherwise the browser's.
 */
export function deviceLanguage(): Language {
  let chosen: string | null = null
  try {
    chosen = window.localStorage.getItem(CHOICE)
  } catch {
    // Storage can be switched off; the browser's language still works.
  }
  return pickLanguage(chosen ? [chosen, ...navigator.languages] : navigator.languages)
}

export function rememberLanguage(language: Language): void {
  try {
    window.localStorage.setItem(CHOICE, language)
  } catch {
    // A convenience only.
  }
}
