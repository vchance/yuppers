import {
  HELP_FIGURES,
  HELP_TOPICS,
  isHelpTopic,
  type HelpBlock,
  type HelpTopic,
  type HelpWording,
  type Language,
} from '@yuppers/shared'
import { useEffect, useState, type MouseEvent, type ReactNode } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { loadHelp } from '../app/wording'
import { PageHeading } from '../components/ui'

/*
 * The help pages: `/help` lists the topics, `/help/{topic}` is one of them.
 * Anyone can read them, signed in or not. Their text is fetched in the
 * language on screen, from a file only these pages load.
 */

/** The id of the n-th section of a topic, the same in every language. */
const sectionId = (index: number) => `section-${index + 1}`

export default function HelpPage({ topic }: { topic: string | null }) {
  const { wording, language } = useI18n()
  const [loaded, setLoaded] = useState<{ language: Language; help: HelpWording } | null>(null)
  // The language whose text could not be fetched, if any.
  const [failed, setFailed] = useState<Language | null>(null)

  useEffect(() => {
    let cancelled = false
    loadHelp(language).then(
      (help) => {
        if (!cancelled) setLoaded({ language, help })
      },
      () => {
        if (!cancelled) setFailed(language)
      },
    )
    return () => {
      cancelled = true
    }
  }, [language])

  // Until the new language's text arrives, the old one stays, as the rest
  // of the app does; on a first visit, there is nothing to show yet.
  if (!loaded) {
    return failed === language ? (
      <p className="notice">{wording.service.unreachable}</p>
    ) : (
      <p>{wording.common.loading}</p>
    )
  }
  const { help } = loaded
  if (topic === null) return <Contents help={help} />
  if (!isHelpTopic(topic)) return <NoSuchTopic help={help} />
  return <Topic key={topic} help={help} topic={topic} />
}

/** Fills in the figures a help message may name, such as `{closeDays}`. */
function useText() {
  const { fmt } = useI18n()
  return (message: string) => fmt(message, HELP_FIGURES)
}

function Contents({ help }: { help: HelpWording }) {
  const { wording, language } = useI18n()
  return (
    <div className="help">
      <PageHeading>{help.title}</PageHeading>
      <p>{help.intro}</p>
      <TopicList help={help} current={null} />
      {/* What we collect and keep, and on what terms: beside the topics. */}
      <p className="learn-more legal-links">
        <Link to={paths.legal('privacy', language)}>{wording.privacy.policy}</Link>
        <Link to={paths.legal('terms', language)}>{wording.termsOfUse.document}</Link>
      </p>
    </div>
  )
}

/** Every topic, as a table of contents: on the first page, and under each topic. */
function TopicList({ help, current }: { help: HelpWording; current: HelpTopic | null }) {
  const text = useText()
  return (
    <nav aria-labelledby="help-topics" className="help-topics">
      <h2 id="help-topics">{help.topicsHeading}</h2>
      <ol className="plain">
        {HELP_TOPICS.map((id) => (
          <li key={id}>
            <Link to={paths.help(id)} aria-current={id === current ? 'page' : undefined}>
              {help.topics[id].title}
            </Link>
            {current === null && <p className="hint">{text(help.topics[id].summary)}</p>}
          </li>
        ))}
      </ol>
      {current !== null && (
        <p>
          <Link to={paths.help()}>{help.allTopics}</Link>
        </p>
      )}
    </nav>
  )
}

interface Section {
  heading: string | null
  blocks: HelpBlock[]
}

/** A topic's blocks, split into sections at each heading. What comes before the first heading has none. */
function sectionsOf(blocks: readonly HelpBlock[]): Section[] {
  const sections: Section[] = [{ heading: null, blocks: [] }]
  for (const block of blocks) {
    if ('h' in block) sections.push({ heading: block.h, blocks: [] })
    else sections[sections.length - 1].blocks.push(block)
  }
  return sections.filter((section) => section.heading !== null || section.blocks.length > 0)
}

function Topic({ help, topic }: { help: HelpWording; topic: HelpTopic }) {
  const text = useText()
  const page = help.topics[topic]
  const sections = sectionsOf(page.blocks)
  const headed = sections.filter((section) => section.heading !== null)

  // A link to a section from elsewhere arrives before the text does, so the
  // browser could not go to it; once the text is here, this does.
  useEffect(() => {
    const target = window.location.hash.slice(1)
    if (target) goTo(target)
  }, [])

  // Each headed section's id, in order, the same as the list above links to.
  let headings = 0
  const ids = sections.map((section) => (section.heading === null ? null : sectionId(headings++)))
  return (
    <article className="help">
      <PageHeading>{page.title}</PageHeading>
      <p className="help-summary">{text(page.summary)}</p>
      {headed.length > 1 && (
        <nav aria-labelledby="help-on-this-page" className="help-sections">
          <h2 id="help-on-this-page">{help.onThisPage}</h2>
          <ul>
            {headed.map((section, index) => (
              <li key={index}>
                <a href={`#${sectionId(index)}`} onClick={follow(sectionId(index))}>
                  {section.heading}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      )}
      {sections.map((section, index) => {
        const body = section.blocks.map((block, at) => <Block key={at} block={block} text={text} />)
        const id = ids[index]
        if (section.heading === null || id === null) return <div key={index}>{body}</div>
        return (
          <section key={index} aria-labelledby={id}>
            <h2 id={id} tabIndex={-1}>
              {section.heading}
            </h2>
            {body}
          </section>
        )
      })}
      <TopicList help={help} current={topic} />
    </article>
  )
}

function Block({ block, text }: { block: HelpBlock; text: (message: string) => string }): ReactNode {
  if ('p' in block) return <p>{text(block.p)}</p>
  if ('ul' in block) {
    return (
      <ul>
        {block.ul.map((item, index) => (
          <li key={index}>{text(item)}</li>
        ))}
      </ul>
    )
  }
  if ('ol' in block) {
    return (
      <ol>
        {block.ol.map((item, index) => (
          <li key={index}>{text(item)}</li>
        ))}
      </ol>
    )
  }
  return null
}

/** Moves to a section and puts the keyboard there, so reading and tabbing go on from it. */
function goTo(id: string) {
  const heading = document.getElementById(id)
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
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
    window.setTimeout(() => goTo(id))
  }
}

function NoSuchTopic({ help }: { help: HelpWording }) {
  const { wording } = useI18n()
  return (
    <div className="help">
      <PageHeading>{wording.common.notFoundTitle}</PageHeading>
      <p>{wording.common.notFoundBody}</p>
      <p>
        <Link to={paths.help()}>{help.allTopics}</Link>
      </p>
    </div>
  )
}
