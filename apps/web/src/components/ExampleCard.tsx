import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'

/**
 * The card on the empty home page that shows what a yup looks like and opens
 * the sample (DESIGN.md §4.3). It is described by its words, not by colour,
 * and it leaves the page once the person has a yup of their own, since it is
 * only drawn where there are none.
 */
export function ExampleCard() {
  const { wording, fmt, language } = useI18n()
  const w = wording.sample
  return (
    <section className="card example-card" aria-labelledby="example-card-heading">
      <h2 id="example-card-heading">{w.homeHeading}</h2>
      <p>{w.homeBody}</p>
      <p className="example-card-deal">
        <strong>{fmt(w.cardWith, { name: w.partyB })}</strong>
        <span className="tag">{w.cardState}</span>
        <span className="hint">{w.cardDetail}</span>
      </p>
      <div className="actions">
        <Link className="button" to={paths.example(language)}>
          {w.see}
        </Link>
      </div>
    </section>
  )
}
