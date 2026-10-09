import { useI18n } from '../app/context'
import { Mark } from './Mark'

import './brand.css'

/*
 * The product's name as it is drawn: the mark beside the whole word, lower
 * case in the display face, with the trademark set up small beside it
 * (`.wordmark` in `brand.css`). The word is always whole, never "yup" on its
 * own. To assistive technology the mark and the word are one image, named for
 * the product; the mark says nothing on its own and is hidden (`Mark`).
 */

/**
 * The branded opening of a screen that stands for the product itself, before
 * anyone is signed in: the mark at a generous size over the wordmark, the two
 * party colours as a short band, and one line saying what Yuppers is. It is
 * not a heading: the screen's own heading follows it.
 */
export function BrandHero() {
  const { wording } = useI18n()
  return (
    <div className="brand-hero">
      <div className="brand-lockup" role="img" aria-label={wording.productName}>
        <Mark className="brand-hero-mark" />
        <span className="wordmark">{wording.productName}</span>
      </div>
      <span className="tandem-band" aria-hidden="true" />
      <p className="tagline">{wording.tagline}</p>
    </div>
  )
}
