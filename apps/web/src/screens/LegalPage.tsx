import {
  defaultLanguage,
  languages,
  LEGAL_EFFECTIVE_DATES,
  legalEffectiveDate,
  legalInline,
  legalSections,
  staticPagePath,
  type Language,
  type LegalBlock,
  type LegalDocument,
  type LegalWording,
} from '@yuppers/shared'
import { useEffect, useState, type MouseEvent, type ReactNode } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { replaceAddress } from '../app/router'
import { paths } from '../app/routes'
import { loadedLegal, loadLegal } from '../app/wording'
import { PageHeading } from '../components/ui'

/*
 * The privacy policy and the terms, at `/{document}` and
 * `/{language}/{document}`, open to anyone. The text is fetched in the
 * language on screen, from a file only this page loads, and laid out as the
 * static page the build writes at the same address (`build/legal-pages.ts`):
 * the same sections under the same anchors, so a link such as
 * `/privacy#text-messages` lands on the same section whether or not scripts
 * run.
 *
 * The address follows the language shown, so that it can be shared or
 * reloaded and say the same thing: `/es/privacy` while it is read in
 * Spanish.
 */

interface Loaded {
  language: Language
  wording: LegalWording
}

export default function LegalPage({ document }: { document: LegalDocument }) {
  const { wording, language } = useI18n()
  const [loaded, setLoaded] = useState<Loaded | null>(() => {
    const found = loadedLegal(document, language)
    return found ? { language, wording: found } : null
  })
  // The language whose text could not be fetched, if any.
  const [failed, setFailed] = useState<Language | null>(null)

  useEffect(() => {
    if (loaded?.language === language) return
    let cancelled = false
    loadLegal(document, language).then(
      (found) => {
        if (!cancelled) setLoaded({ language, wording: found })
      },
      () => {
        if (!cancelled) setFailed(language)
      },
    )
    return () => {
      cancelled = true
    }
  }, [document, language, loaded?.language])

  useEffect(() => {
    const wanted = paths.legal(document, language)
    if (window.location.pathname !== wanted || window.location.search) replaceAddress(wanted)
  }, [document, language])

  // A link to a section can arrive before the text does, so the browser
  // could not go to it; once the text is here, this does.
  const ready = loaded !== null
  useEffect(() => {
    const target = window.location.hash.slice(1)
    if (ready && target) goTo(target)
  }, [ready])

  // Until the new language's text arrives, the old one stays, as the rest of
  // the app does; on a first visit, there is nothing to show yet.
  if (!loaded) {
    return failed === language ? (
      <p className="notice">{wording.service.unreachable}</p>
    ) : (
      <p>{wording.common.loading}</p>
    )
  }
  return <Document document={document} wording={loaded.wording} language={loaded.language} />
}

interface Context {
  document: LegalDocument
  wording: LegalWording
  language: Language
}

function Document(context: Context) {
  const { document, wording, language } = context
  const { setLanguage } = useI18n()
  const sections = legalSections(document, wording)
  const [beforeDate, afterDate = ''] = wording.effective.split('{effectiveDate}')

  return (
    <article className="legal">
      <PageHeading>{wording.title}</PageHeading>
      <p className="legal-note">
        <Inline message={wording.note} context={context} />
      </p>
      <p className="hint">
        <Inline message={beforeDate} context={context} />
        <time dateTime={LEGAL_EFFECTIVE_DATES[document]}>{legalEffectiveDate(document, language)}</time>
        <Inline message={afterDate} context={context} />
      </p>
      <nav aria-label={wording.otherLanguages} className="legal-languages">
        <ul className="plain">
          {languages.map((info) => (
            <li key={info.code}>
              <a
                href={paths.legal(document, info.code)}
                lang={info.code}
                hrefLang={info.code}
                aria-current={info.code === language ? 'page' : undefined}
                onClick={(event) => {
                  if (!plainClick(event)) return
                  // The same page in another language: the app's language
                  // changes, and the address with it.
                  event.preventDefault()
                  setLanguage(info.code)
                }}
              >
                {info.name}
              </a>
            </li>
          ))}
        </ul>
      </nav>
      <nav aria-labelledby="legal-contents" className="help-sections">
        <h2 id="legal-contents">{wording.contents}</h2>
        <ol>
          {sections.map(({ id, section }) => (
            <li key={id}>
              <a href={`#${id}`} onClick={follow(id)}>
                <Inline message={section.title} context={context} />
              </a>
            </li>
          ))}
        </ol>
      </nav>
      {sections.map(({ id, section }) => (
        <section key={id} aria-labelledby={id}>
          <h2 id={id} tabIndex={-1}>
            <Inline message={section.title} context={context} />
          </h2>
          {section.blocks.map((block, index) => (
            <Block key={index} block={block} context={context} />
          ))}
        </section>
      ))}
    </article>
  )
}

function Block({ block, context }: { block: LegalBlock; context: Context }): ReactNode {
  if ('h' in block) {
    return (
      <h3>
        <Inline message={block.h} context={context} />
      </h3>
    )
  }
  if ('p' in block) {
    return (
      <p>
        <Inline message={block.p} context={context} />
      </p>
    )
  }
  return (
    <ul>
      {block.ul.map((item, index) => (
        <li key={index}>
          <Inline message={item} context={context} />
        </li>
      ))}
    </ul>
  )
}

/** A message, filled in, with its addresses and links as links and its strong text strong. */
function Inline({ message, context }: { message: string; context: Context }) {
  const { document, wording, language } = context
  return legalInline(message, language, document, wording).map((piece, index) => {
    if ('email' in piece) {
      return (
        <a key={index} href={`mailto:${piece.email}`}>
          {piece.email}
        </a>
      )
    }
    if ('page' in piece) {
      // A page of its own, not the app's: loaded whole.
      return (
        <a key={index} href={staticPagePath(piece.page, language, defaultLanguage)}>
          {piece.text}
        </a>
      )
    }
    if ('document' in piece) {
      const path = paths.legal(piece.document, language)
      return (
        <Link key={index} to={`${path}${piece.section ? `#${piece.section}` : ''}`}>
          {piece.text}
        </Link>
      )
    }
    return piece.strong ? <strong key={index}>{piece.text}</strong> : piece.text
  })
}

function plainClick(event: MouseEvent<HTMLAnchorElement>): boolean {
  return (
    event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey
  )
}

/** Moves to a section and puts the keyboard there, so reading and tabbing go on from it. */
function goTo(id: string) {
  const heading = window.document.getElementById(id)
  if (!heading) return
  heading.scrollIntoView()
  heading.focus({ preventScroll: true })
}

/**
 * Following a link to a section: the browser changes the address and
 * scrolls, and this then gives the heading the focus, which not every
 * browser does on its own.
 */
function follow(id: string) {
  return (event: MouseEvent<HTMLAnchorElement>) => {
    if (plainClick(event)) window.setTimeout(() => goTo(id))
  }
}
