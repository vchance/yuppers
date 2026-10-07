/*
 * The two fonts the invitation page's first screen is mostly drawn in,
 * preloaded so they arrive with the stylesheet rather than after it
 * (DESIGN.md §13.5). Instrument Sans 400 is the running text and Gabarito 900
 * the wordmark and the headings. The page also uses Instrument Sans 700 (the
 * field's label) and Gabarito 700 (the ™ beside the wordmark); those few
 * words swap in when their fonts arrive, as everything else does
 * (`font-display: swap` in `src/index.css`), and are left to the stylesheet
 * to keep the page within its budget (`scripts/check-budget.mjs`, which
 * counts what is preloaded).
 *
 * Only the invitation pages, `{language}/i/index.html`: a person opening a
 * link has never loaded the app, while the app's own entry page is mostly
 * opened by people who have, and its fonts are in their cache.
 */
export const PRELOADED_FONTS = [
  'src/fonts/instrument-sans/InstrumentSans-Regular.woff2',
  'src/fonts/gabarito/Gabarito-Black.woff2',
] as const

/** What the build knows of an emitted asset: its file and where it came from. */
export interface EmittedAsset {
  fileName: string
  originalFileNames: readonly string[]
}

/**
 * The address each preloaded font is served at, hash and all, from the
 * assets the build emitted. Fails the build if one is missing, so a renamed
 * font cannot quietly stop being preloaded.
 */
export function preloadedFontHrefs(assets: Iterable<EmittedAsset>, base = '/'): string[] {
  const all = [...assets]
  return PRELOADED_FONTS.map((source) => {
    const asset = all.find((candidate) =>
      candidate.originalFileNames.some(
        (original) => original === source || original.endsWith(`/${source}`),
      ),
    )
    if (!asset) throw new Error(`the build emitted no ${source} to preload`)
    return `${base}${asset.fileName}`
  })
}

/**
 * The page with a preload link for each font, after its stylesheet. Same
 * origin, so the Content-Security-Policy's `default-src 'self'` covers them;
 * `crossorigin` because fonts are always fetched in CORS mode, and a preload
 * without it would be fetched twice.
 */
export function withFontPreloads(html: string, hrefs: readonly string[]): string {
  const end = /(\n[ \t]*)<\/head>/
  if (!end.test(html)) throw new Error('the page has no </head> to preload fonts in')
  return html.replace(end, (_, line: string) => {
    const links = hrefs.map(
      (href) => `${line}  <link rel="preload" as="font" type="font/woff2" crossorigin href="${href}">`,
    )
    return `${links.join('')}${line}</head>`
  })
}
