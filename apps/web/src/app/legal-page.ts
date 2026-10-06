import type { LegalDocument } from '@yuppers/shared'
import type { ComponentType } from 'react'

/*
 * The privacy policy's and the terms' page is a chunk of its own, like
 * every page but the invitation page. Opened directly at its address, the
 * service answers with the document already written into the page
 * (`build/legal-pages.ts`), and the app then draws its own page over it;
 * `main.tsx` fetches the chunk and the text before the app first draws, so
 * the document is replaced by itself and not, for a moment, by "Loading…".
 */

type LegalPageComponent = ComponentType<{ document: LegalDocument }>

let loaded: LegalPageComponent | null = null

export function loadLegalPage(): Promise<{ default: LegalPageComponent }> {
  return import('../screens/LegalPage').then((module) => {
    loaded = module.default
    return module
  })
}

/** The page, if its chunk has already arrived. */
export function loadedLegalPage(): LegalPageComponent | null {
  return loaded
}
