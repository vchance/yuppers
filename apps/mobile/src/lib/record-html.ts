import {
  documentOf,
  directionOf,
  eventMessage,
  moneyIds,
  noteKind,
  recordDays,
  recordMoments,
  statusWording,
  summarizeRecord,
  summaryText,
  termsOfRevision,
  verificationText,
  type ClosedReason,
  type I18n,
  type RecordDocument,
  type RecordRevision,
} from '@yuppers/shared';

/*
 * A record as one HTML page, for the device to print to a PDF (DESIGN.md
 * §14.1): the plain summary first, then the record in full, as the record
 * screen shows it. Everything in it is escaped, since much of it is what the
 * parties wrote; the page allows no script and loads nothing, so it is only
 * ever text and its own styles.
 */

const escape = (text: string) =>
  text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');

/** A paragraph of the product's words. */
const p = (text: string, kind?: 'hint' | 'status' | 'label') =>
  `<p${kind ? ` class="${kind}"` : ''}>${escape(text)}</p>`;
/** What a party wrote, set apart as theirs and never turned around by what is near it. */
const written = (text: string) => `<p class="written" dir="auto">${escape(text)}</p>`;
const heading = (level: 1 | 2 | 3, text: string) => `<h${level}>${escape(text)}</h${level}>`;

const STYLE = `
  body { font: 11pt/1.45 -apple-system, Roboto, "Helvetica Neue", Arial, sans-serif; color: #111; margin: 0; }
  h1 { font-size: 18pt; margin: 0 0 6pt; }
  h2 { font-size: 14pt; margin: 18pt 0 6pt; break-after: avoid; }
  h3 { font-size: 12pt; margin: 12pt 0 4pt; break-after: avoid; }
  p { margin: 0 0 5pt; }
  .hint { color: #444; font-size: 10pt; }
  .status { font-weight: 600; }
  .label { font-weight: 600; margin-top: 4pt; }
  .written { border-left: 2.5pt solid #555; padding-left: 7pt; font-family: Georgia, serif; white-space: pre-wrap; overflow-wrap: anywhere; }
  [dir="rtl"] .written { border-left: 0; border-right: 2.5pt solid #555; padding-left: 0; padding-right: 7pt; }
  .entry { border-top: 0.5pt solid #999; padding-top: 5pt; margin-top: 5pt; break-inside: avoid; }
  .summary { border: 1pt solid #999; padding: 8pt 10pt; }
  .fingerprint { overflow-wrap: anywhere; }
`;

/** The record, summary first, as an HTML document for printing. */
export function recordHtml(
  record: RecordDocument,
  i18n: Pick<I18n, 'language' | 'wording' | 'fmt' | 'day' | 'money'>,
): string {
  const { wording, fmt, day, money: moneyText, language } = i18n;
  const w = wording.record;
  const { exchange, parties } = record;
  const when = recordMoments(language, exchange.timezone);
  const code = exchange.display_code;
  const money = moneyIds(record.revisions.map(termsOfRevision));
  const out: string[] = [];

  out.push(heading(1, fmt(w.title, { code })));
  out.push(p(fmt(w.madeFor, { name: parties[record.prepared_for], date: when(record.generated_at) }), 'hint'));
  out.push(p(fmt(w.timesIn, { timezone: exchange.timezone }), 'hint'));

  // The plain summary.
  const days = recordDays(language, exchange.timezone);
  const summary = summaryText(summarizeRecord(record), i18n, exchange.currency, days);
  out.push('<section class="summary">');
  out.push(heading(2, summary.heading));
  out.push(p(summary.intro, 'hint'), p(summary.between), p(summary.basis));
  for (const side of summary.sides) {
    out.push(heading(3, side.heading));
    if (side.nothing) out.push(p(side.nothing));
    for (const item of side.items) {
      out.push('<div class="entry">', written(item.description));
      for (const detail of item.details) out.push(p(detail));
      if (item.outcome) out.push(p(item.outcome, 'status'));
      out.push('</div>');
    }
  }
  for (const line of [...summary.signed, ...summary.standing]) out.push(p(line));
  out.push('</section>');

  // How it stands, as the record screen says it.
  out.push(heading(2, w.summaryHeading));
  out.push(p(exchange.closed_outcome ? wording.outcomes[exchange.closed_outcome] : wording.states[exchange.state], 'status'));
  out.push(p(fmt(wording.home.reference, { code })));
  out.push(heading(3, wording.terms.partiesHeading));
  for (const slot of ['A', 'B'] as const) if (parties[slot]) out.push(written(parties[slot]));
  const reason = exchange.closed_reason;
  if (reason && Object.hasOwn(w.closedReasons, reason)) out.push(p(w.closedReasons[reason as ClosedReason]));
  out.push(p(fmt(w.started, { date: when(exchange.created_at) })));
  if (exchange.closed_at) out.push(p(fmt(w.closedOn, { date: when(exchange.closed_at) })));
  out.push(
    p(exchange.in_force_revision ? fmt(w.agreementIs, { number: exchange.in_force_revision.sequence }) : w.agreementNone),
  );
  if (exchange.open_revision) out.push(p(fmt(w.waiting, { number: exchange.open_revision.sequence })));
  if (exchange.end_proposed_by) out.push(p(fmt(w.endProposed, { name: parties[exchange.end_proposed_by] })));
  if (exchange.close_requested_by && exchange.close_requested_at) {
    out.push(
      p(fmt(w.closeRequested, { name: parties[exchange.close_requested_by], date: when(exchange.close_requested_at) })),
    );
  }

  out.push(heading(2, w.aboutHeading));
  out.push(p(w.export.about), p(w.export.signatures), p(w.export.statements), p(w.export.contentHash));

  if (record.contributions.length > 0) {
    out.push(heading(2, w.itemsHeading));
    for (const contribution of record.contributions) {
      out.push('<div class="entry">', written(contribution.description));
      out.push(p(fmt(w.itemFrom, { name: parties[contribution.from] })));
      out.push(p(statusWording(wording, contribution.status, money.has(contribution.id)), 'status'));
      if (contribution.since) out.push(p(fmt(w.since, { date: when(contribution.since) }), 'hint'));
      out.push('</div>');
    }
  }

  if (record.revisions.length === 0) out.push(p(w.versionsNone));
  for (const revision of record.revisions) out.push(version(revision));

  out.push(heading(2, w.eventsHeading));
  if (record.events.length === 0) out.push(p(w.historyEmpty));
  for (const event of record.events) {
    const { message, values } = eventMessage(event, w.events, null, parties, money);
    out.push('<div class="entry">', p(when(event.at), 'hint'), p(fmt(message, values)));
    if (event.contribution?.description) out.push(written(event.contribution.description));
    if (event.note) out.push(p(w.noteLabels[noteKind(event)], 'label'), written(event.note));
    out.push('</div>');
  }
  if (record.history_chain) out.push(p(record.history_chain, 'hint'));

  /** One version that was sent, in full, with its signatures. */
  function version(revision: RecordRevision): string {
    const parts: string[] = [];
    const { standing } = revision;
    const signed = documentOf(revision);
    const name = (slot: 'A' | 'B') => signed.parties[slot];
    parts.push(heading(2, fmt(w.versionHeading, { number: revision.sequence })));
    parts.push(
      p(fmt(w.versionSent, { name: name(revision.author), date: when(revision.sent_at), expires: when(revision.expires_at) })),
    );
    if (revision.answers) parts.push(p(fmt(w.versionAnswers, { number: revision.answers.sequence })));
    parts.push(p(w.versionStatus[standing.status], 'status'));
    parts.push(p(fmt(w.since, { date: when(standing.since) }), 'hint'));
    if (standing.replaced_by) parts.push(p(fmt(w.versionReplacedBy, { number: standing.replaced_by.sequence }), 'hint'));
    if (revision.note) parts.push(heading(3, w.noteLabels.message), written(revision.note));

    const t = wording.terms;
    const terms = termsOfRevision(revision);
    parts.push(p(t.ownWords, 'hint'));
    parts.push(heading(3, t.partiesHeading), written(terms.party_a_name), written(terms.party_b_name));
    if (terms.terms.trim() !== '') parts.push(heading(3, t.termsHeading), written(terms.terms));
    for (const slot of ['A', 'B'] as const) {
      const provided = terms.contributions.filter((contribution) => contribution.from === slot);
      parts.push(heading(3, fmt(t.otherProvides, { name: name(slot) })));
      if (provided.length === 0) parts.push(p(t.nothing));
      for (const contribution of provided) {
        parts.push('<div class="entry">');
        parts.push(
          p(`${wording.contributionTypes[contribution.type]} · ${contribution.required ? t.required : t.optional}`, 'hint'),
        );
        parts.push(written(contribution.description));
        if (contribution.amount_minor != null) {
          parts.push(p(fmt(t.amount, { amount: moneyText(contribution.amount_minor, signed.currency) })));
        }
        if (contribution.type === 'MONEY') parts.push(p(t.moneyOutside, 'hint'));
        if (contribution.quantity) {
          parts.push(
            p(
              contribution.quantity.unit
                ? fmt(t.quantityWithUnit, { amount: contribution.quantity.amount, unit: contribution.quantity.unit })
                : fmt(t.quantity, { amount: contribution.quantity.amount }),
            ),
          );
        }
        const due = contribution.due;
        if (due.kind === 'DATE') parts.push(p(fmt(t.dueOnDate, { date: day(due.date) })));
        else if (due.kind === 'ON_AGREEMENT') parts.push(p(t.dueOnAgreement));
        else {
          const awaited = terms.contributions.find((other) => other.id === due.contribution);
          parts.push(p(fmt(t.dueAfter, { description: awaited?.description ?? '' })));
        }
        if (contribution.completion_criteria) {
          parts.push(p(t.criteriaLabel, 'label'), written(contribution.completion_criteria));
        }
        parts.push('</div>');
      }
    }
    if (terms.contributions.some((contribution) => contribution.due.kind === 'DATE')) {
      parts.push(p(fmt(t.timezone, { timezone: signed.timezone }), 'hint'));
    }
    parts.push(`<p class="hint fingerprint">${escape(fmt(t.fingerprint, { hash: revision.content_hash }))}</p>`);

    parts.push(heading(3, w.signaturesHeading));
    for (const signature of revision.signatures) {
      parts.push('<div class="entry">');
      parts.push(p(fmt(w.versionSignedBy, { name: signature.name, date: when(signature.signed_at) })));
      parts.push(proof(signature));
      parts.push('</div>');
    }
    for (const signature of revision.void_signatures ?? []) {
      parts.push('<div class="entry">');
      parts.push(
        p(fmt(wording.claimant.voidSignature, { date: when(signature.signed_at), since: when(signature.void_since) })),
      );
      parts.push(proof(signature));
      parts.push('</div>');
    }
    return parts.join('');
  }

  /** What a signature rests on. */
  type Signed =
    | RecordRevision['signatures'][number]
    | NonNullable<RecordRevision['void_signatures']>[number];
  function proof(signature: Signed) {
    return [
      p(fmt(w.verifiedBy, { method: verificationText(signature.verification, w.export.verification) }), 'hint'),
      p(fmt(w.verifiedAt, { date: when(signature.verification.verified_at) }), 'hint'),
      signature.verification.attribution ? p(signature.verification.attribution, 'hint') : '',
      p(fmt(w.consentShown, { version: signature.consent.version, language: signature.consent.language }), 'hint'),
    ].join('');
  }

  return [
    '<!DOCTYPE html>',
    `<html lang="${escape(language)}" dir="${directionOf(language)}">`,
    '<head>',
    '<meta charset="utf-8">',
    // Nothing runs and nothing is fetched: the page is text and its own styles.
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">`,
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    `<title>${escape(fmt(w.title, { code }))}</title>`,
    `<style>${STYLE}</style>`,
    '</head>',
    '<body>',
    out.join('\n'),
    '</body>',
    '</html>',
  ].join('\n');
}
