import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import './index.css'
import { App } from './app/App.tsx'
import { loadLegalPage } from './app/legal-page.ts'
import { matchRoute } from './app/routes.ts'
import { addressLanguage, deviceLanguage, loadLegal, loadWording } from './app/wording.ts'

// Nothing can be shown without wording, so the one language this visitor
// needs is fetched before the first render: the one the link asks for, if
// any, then this device's.
const language = addressLanguage() ?? deviceLanguage()

// The privacy policy's and the terms' pages arrive with the document
// already in them, which the first render replaces; with the page and the
// text here first, it is replaced by itself. Should either fail, the page
// loads them as usual.
const route = matchRoute(window.location.pathname)
const legal =
  route.name === 'legal'
    ? Promise.all([loadLegalPage(), loadLegal(route.document, language)]).catch(() => null)
    : null

Promise.all([loadWording(language), legal]).then(([wording]) => {
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App initialLanguage={language} initialWording={wording} />
    </StrictMode>,
  )
})
