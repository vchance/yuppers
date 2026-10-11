import type { components, RevisionTerms } from '@yuppers/api-client'

import {
  draftFromTerms,
  emptyDraft,
  newContribution,
  type Draft,
  type DraftContribution,
  type ItemExample,
} from './draft'
import type { Wording } from './wording/types'

type ContributionType = components['schemas']['ContributionType']
type Slot = components['schemas']['Slot']

/*
 * Starting a yup (DESIGN.md §4.4): a short list of common situations, each
 * the shape of an agreement and nothing more, then the blank form, then a
 * copy of an earlier yup.
 *
 * A template is a declaration kept here: which items there are, who provides
 * each, its kind, how it is due, whether it is required, and which item it
 * waits on. Every word a person sees is in the wording files
 * (`templates.entries`). A template never fills in text that could be sent:
 * a description, a criterion, a quantity or a unit is an example, shown in
 * grey, and counts as empty until the person writes something of their own.
 * Amounts and dates are always typed. There are no clauses here and none in
 * the wording; the tests keep it so (`templates.test.ts`).
 *
 * Shipped with the apps, not served by the API, and applied once, on the
 * client, into an ordinary draft. The version goes up when a template's
 * shape changes, and is what `started_from` records beside the id.
 */

export type TemplateId =
  | 'job-deposit-balance'
  | 'selling-something'
  | 'swap-no-money'
  | 'lending-item'
  | 'pet-sitting-childcare'
  | 'splitting-cost'

/** How an item of a template is due. `after` is the position of the item it waits for. */
export type TemplateDue = 'SIGNED' | 'DATE' | { after: number }

export interface TemplateItem {
  /** `you` is the author, who provides it; `them` is the other party. */
  from: 'you' | 'them'
  type: ContributionType
  due: TemplateDue
  required: boolean
}

export interface Template {
  id: TemplateId
  /** Goes up when the template's shape changes. */
  version: number
  items: readonly TemplateItem[]
  /**
   * Whether the wording carries a warning for this situation, shown where
   * the template is chosen and above its items (LEGAL_MEMO.md item 47).
   */
  warns?: true
}

/*
 * Held pending counsel (LEGAL_MEMO.md item 46): the design's T3, lending
 * money, is not declared here and has no wording. Nothing in the chooser,
 * the wording or the backend's list of entries mentions it. Declaring it is
 * a change to this file, the wording files and `Entry` in
 * `backend/src/funnel.rs`, after counsel has answered.
 */

/**
 * The templates, in the order the chooser lists them (DESIGN.md §4.4,
 * section 18 item 42): a job, a sale, a swap, lending an item, care, and
 * splitting a cost.
 */
export const TEMPLATES: readonly Template[] = [
  {
    id: 'job-deposit-balance',
    version: 1,
    warns: true,
    items: [
      { from: 'them', type: 'MONEY', due: 'SIGNED', required: true },
      { from: 'you', type: 'SERVICE', due: 'DATE', required: true },
      { from: 'them', type: 'MONEY', due: { after: 1 }, required: true },
    ],
  },
  {
    id: 'selling-something',
    version: 1,
    items: [
      { from: 'you', type: 'ITEM', due: 'DATE', required: true },
      { from: 'them', type: 'MONEY', due: 'DATE', required: true },
    ],
  },
  {
    id: 'swap-no-money',
    version: 1,
    items: [
      { from: 'you', type: 'SERVICE', due: 'DATE', required: true },
      { from: 'them', type: 'SERVICE', due: 'DATE', required: true },
    ],
  },
  {
    id: 'lending-item',
    version: 1,
    items: [
      { from: 'you', type: 'ITEM', due: 'SIGNED', required: true },
      { from: 'them', type: 'ITEM', due: 'DATE', required: true },
    ],
  },
  {
    id: 'pet-sitting-childcare',
    version: 1,
    items: [
      { from: 'you', type: 'SERVICE', due: 'DATE', required: true },
      { from: 'them', type: 'MONEY', due: { after: 0 }, required: true },
    ],
  },
  {
    id: 'splitting-cost',
    version: 1,
    items: [
      { from: 'them', type: 'MONEY', due: 'DATE', required: true },
      { from: 'you', type: 'ITEM', due: 'SIGNED', required: false },
    ],
  },
]

export function templateById(id: string): Template | undefined {
  return TEMPLATES.find((template) => template.id === id)
}

export type TemplateWords = Wording['templates']['entries'][TemplateId]

/** What a yup was started from: a template, the blank form, or a copy of an earlier yup. */
export type StartChoice =
  | { kind: 'template'; template: Template }
  | { kind: 'blank' }
  | { kind: 'copy' }

/**
 * What the service is told once, when the draft is made, and keeps for
 * counting how people begin (`started_from`, DESIGN.md §4.4, "Measurement"):
 * `job-deposit-balance@1`, `blank` or `copy`. It is never part of the draft.
 */
export function startedFrom(choice: StartChoice): string {
  return choice.kind === 'template' ? `${choice.template.id}@${choice.template.version}` : choice.kind
}

/** The grey examples an item of a template starts with, from its wording. */
export function exampleOf(words: TemplateWords['items'][number]): ItemExample {
  const example: ItemExample = { description: words.description }
  if (words.criteria) example.criteria = words.criteria
  if (words.quantity) example.quantity = words.quantity
  if (words.unit) example.unit = words.unit
  return example
}

/**
 * The working copy a template starts: its items, each with the examples in
 * grey, nothing typed. `newId` makes an item's id, which lives and dies with
 * the one exchange. The author is the first party, as in every draft.
 */
export function applyTemplate(
  template: Template,
  words: TemplateWords,
  author: string,
  newId: () => string,
): Draft {
  const ids = template.items.map(() => newId())
  const draft = emptyDraft(author)
  draft.contributions = template.items.map((item, position): DraftContribution => {
    const contribution = newContribution(ids[position], item.from === 'you' ? 'A' : 'B')
    contribution.type = item.type
    contribution.required = item.required
    if (item.due === 'DATE') contribution.due = { kind: 'DATE', date: '' }
    else if (typeof item.due === 'object') {
      contribution.due = { kind: 'AFTER_CONTRIBUTION', contribution: ids[item.due.after] }
    }
    const itemWords = words.items[position]
    if (itemWords) contribution.example = exampleOf(itemWords)
    return contribution
  })
  return draft
}

/**
 * Every item's provider turned round, for when the other person is the one
 * writing: each template is written from one side. Nothing else changes.
 */
export function swapSides(draft: Draft): Draft {
  const flip = (slot: Slot): Slot => (slot === 'A' ? 'B' : 'A')
  return {
    ...draft,
    contributions: draft.contributions.map((item) => ({ ...item, from: flip(item.from) })),
    // Putting a split back restores the item as it was, on the side it is now.
    ...(draft.splits
      ? {
          splits: draft.splits.map((group) => ({
            ...group,
            original: { ...group.original, from: flip(group.original.from) },
          })),
        }
      : {}),
  }
}

/**
 * Whether an item's example is all there is to its description: the person
 * has written nothing of their own, so it counts as empty. (An example is
 * never the description; this is for a screen that wants to say so.)
 */
export function isExampleOnly(item: DraftContribution): boolean {
  return item.example !== undefined && item.description.trim() === ''
}

/**
 * A new draft from an earlier yup's last version (DESIGN.md §4.4, "Copy a
 * previous yup"). What is copied: the author's name as written, the terms,
 * and every item's provider, kind, description, quantity and unit, amount,
 * what makes it done, and whether it is required; "due when signed" and "due
 * once another item is confirmed" are kept, with the links pointing at the
 * new copies, which have ids of their own. What is not: every due date, the
 * message, who the invitation is for, signatures, history and anything
 * recorded. The other party's name is copied only for `samePerson`.
 *
 * `you` is the slot the author held in the yup being copied; the copy is a
 * first proposal, in which the author is always the first party, so a
 * yup written from the second party's side is turned round.
 */
export function draftFromCopy(
  terms: RevisionTerms,
  you: Slot,
  samePerson: boolean,
  fractionDigits: number,
  newId: () => string,
): Draft {
  const copied = draftFromTerms(terms, '', fractionDigits)
  const ids = new Map(copied.contributions.map((item) => [item.id, newId()]))
  const flip = (slot: Slot): Slot => (you === 'A' ? slot : slot === 'A' ? 'B' : 'A')
  return {
    format: 1,
    base: null,
    partyA: you === 'A' ? copied.partyA : copied.partyB,
    partyB: samePerson ? (you === 'A' ? copied.partyB : copied.partyA) : '',
    terms: copied.terms,
    note: '',
    contributions: copied.contributions.map((item) => {
      const due = item.due
      return {
        ...item,
        id: ids.get(item.id) ?? newId(),
        from: flip(item.from),
        due:
          due.kind === 'DATE'
            ? { kind: 'DATE', date: '' }
            : due.kind === 'AFTER_CONTRIBUTION'
              ? { kind: 'AFTER_CONTRIBUTION', contribution: ids.get(due.contribution) ?? '' }
              : due,
      }
    }),
  }
}
