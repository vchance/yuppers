import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, test } from 'vitest'

import { buildTerms, readDraft } from './draft'
import { languages, wordingFor } from './language'
import {
  applyTemplate,
  draftFromCopy,
  isExampleOnly,
  startedFrom,
  swapSides,
  templateById,
  TEMPLATES,
  type Template,
} from './templates'

const read = (relative: string) =>
  readFileSync(fileURLToPath(new URL(relative, import.meta.url)), 'utf8')

let counter = 0
const newId = () => `id-${++counter}`

function words(code: 'en' | 'es', template: Template) {
  return wordingFor(code).templates.entries[template.id]
}

describe('the declarations', () => {
  test('are a short set with ids and versions of their own', () => {
    expect(TEMPLATES.map((template) => `${template.id}@${template.version}`)).toEqual([
      'job-deposit-balance@1',
      'selling-something@1',
      'swap-no-money@1',
      'lending-item@1',
      'pet-sitting-childcare@1',
      'splitting-cost@1',
    ])
    for (const template of TEMPLATES) {
      expect(template.id).toMatch(/^[a-z][a-z0-9-]{0,39}$/)
      expect(Number.isInteger(template.version) && template.version >= 1).toBe(true)
      expect(templateById(template.id)).toBe(template)
    }
  })

  test('hold structure that stands up: links point backwards at other items, something is required', () => {
    for (const template of TEMPLATES) {
      expect(template.items.some((item) => item.required)).toBe(true)
      template.items.forEach((item, position) => {
        if (typeof item.due === 'object') {
          expect(item.due.after).toBeGreaterThanOrEqual(0)
          expect(item.due.after).toBeLessThan(template.items.length)
          expect(item.due.after).not.toBe(position)
        }
      })
    }
  })

  test('are told to the service in the form it accepts, for the ones that exist', () => {
    expect(startedFrom({ kind: 'template', template: TEMPLATES[0] })).toBe('job-deposit-balance@1')
    expect(startedFrom({ kind: 'blank' })).toBe('blank')
    expect(startedFrom({ kind: 'copy' })).toBe('copy')
  })

  test('are the entries the service counts, and the held lending template is nowhere', () => {
    const funnel = read('../../../backend/src/funnel.rs')
    for (const template of TEMPLATES) expect(funnel).toContain(`"${template.id}"`)
    // Held pending counsel (LEGAL_MEMO.md item 46).
    const everything = [
      TEMPLATES.map((template) => template.id).join(' '),
      JSON.stringify(wordingFor('en').templates),
      JSON.stringify(wordingFor('es').templates),
      funnel,
    ].join('\n')
    expect(everything).not.toMatch(/lending-money|lending money|prestar dinero|promissory|loan/i)
  })
})

describe('the wording', () => {
  for (const { code } of languages) {
    test(`in ${code} has every template, with an example for every item and no more`, () => {
      const entries = wordingFor(code).templates.entries
      expect(Object.keys(entries).sort()).toEqual(TEMPLATES.map((template) => template.id).sort())
      for (const template of TEMPLATES) {
        const entry = words(code, template)
        expect(entry.items).toHaveLength(template.items.length)
        for (const text of [entry.name, entry.summary, entry.hint]) expect(text.trim()).not.toBe('')
        entry.items.forEach((item, position) => {
          expect(item.description.trim()).not.toBe('')
          // Examples read as examples, so they cannot be taken for the description.
          const marker = code === 'en' ? /^e\.g\. / : /^p\. ej\. /
          for (const text of [item.description, item.criteria, item.quantity, item.unit]) {
            if (text !== undefined) expect(text).toMatch(marker)
          }
          // Money has an amount, typed, and no quantity.
          if (template.items[position].type === 'MONEY') {
            expect(item.quantity).toBeUndefined()
            expect(item.unit).toBeUndefined()
          }
        })
      }
    })
  }

  test('says the same things in each language: examples and criteria for the same items', () => {
    for (const template of TEMPLATES) {
      const shape = (code: 'en' | 'es') =>
        words(code, template).items.map((item) => [
          item.criteria !== undefined,
          item.quantity !== undefined,
          item.unit !== undefined,
        ])
      expect(shape('es')).toEqual(shape('en'))
    }
  })

  test('warns about New Jersey home-improvement contracts on the job, in both languages, and only there', () => {
    for (const template of TEMPLATES) {
      for (const code of ['en', 'es'] as const) {
        const warning = words(code, template).warning
        if (template.warns) {
          expect(warning).toMatch(code === 'en' ? /New Jersey/ : /Nueva Jersey/)
          expect(warning).toContain('$500')
        } else expect(warning).toBeUndefined()
      }
    }
    expect(TEMPLATES.filter((template) => template.warns).map((template) => template.id)).toEqual([
      'job-deposit-balance',
    ])
  })

  test('names what yups are not for, and that Yuppers is not a lawyer', () => {
    const en = wordingFor('en').templates
    expect(en.notFor).toBe(
      'Not for: anything illegal; bets; anyone under 18; selling, renting or giving away land or a home, or any lease of a place to live; wills, trusts, powers of attorney, health-care or family-law matters; support between unmarried partners; collateral or security for a debt; hiring an employee; firearms, prescription drugs, alcohol or cannabis.',
    )
    expect(en.notForAdvice).toBe(
      'Yuppers isn’t a lawyer and doesn’t give legal advice. If you’re not sure what to write, ask one.',
    )
    const es = wordingFor('es').templates
    expect(es.notFor).toMatch(/^No sirve para:/)
    expect(es.notForAdvice).toMatch(/abogado/)
  })

  // Structure and examples only (DESIGN.md §4.4; counsel's memo). The words a
  // template may not use are the words of a clause, of a promise about the
  // law, and of telling a person what to agree. This is a list to grow, and
  // every hint and example is in it.
  const CLAUSE_LIKE = {
    en: /\b(shall|must|should|hereby|thereof|warrant\w*|liable|liability|indemnif\w*|governing law|late fee|penalt\w*|interest|obligat\w*|default\w*|binding|enforc\w*|breach|forfeit\w*|make sure|be sure|ensure|remember to|don’t forget|always|never|recommend\w*|advis\w*|suggest\w*|you need|you have to|you ought|legal|lawful|law|loan\w*|borrow\w*|lend(er)?s?\b|the borrower)\b/i,
    es: /\b(deber[aá]n?|deben?|deber[ií]a\w*|hereby|garant[ií]a\w*|inter[eé]s(es)?|penaliza\w*|recargo|responsable|responsabilidad|cl[aá]usula\w*|asegúrate|aseg[uú]rese|recuerda|no olvides|siempre|nunca|recomend\w*|aconsej\w*|sugier\w*|tienes que|tiene que|hay que|ley aplicable|vinculante|legal\w*|pr[eé]stamo\w*|prestatario|incumpl\w*)\b/i,
  }
  // The first word of a sentence in the imperative: telling the person what to do.
  const IMPERATIVE = {
    en: /^(add|remove|write|make|include|ask|pay|agree|split|use|leave|enter|choose|put|keep|check|tell|give|take|hold|set|be|do|don’t|let|try|confirm|record|state|name|list|note|mark|decide|consider|read|sign|send|get|bring|return|promise)\b/i,
    es: /^(a[ñn]ade|quita|escribe|escr[ií]belo|haz|incluye|pregunta|paga|acuerda|divide|usa|deja|ingresa|elige|pon|guarda|revisa|di|dile|da|toma|sostén|fija|sé|confirma|registra|indica|nombra|anota|marca|decide|considera|lee|firma|env[ií]a|trae|devuelve|promete|consulta)\b/i,
  }

  /** Everything a template says about itself, other than what counsel wrote word for word. */
  function templateTexts(code: 'en' | 'es'): [string, string][] {
    const w = wordingFor(code).templates
    const texts: [string, string][] = [
      ['blank.summary', w.blank.summary],
      ['copy.summary', w.copy.summary],
      ['chooserIntro', w.chooserIntro],
      ['bandExamples', w.bandExamples],
      ['swapSidesHint', w.swapSidesHint],
      ['copyIntro', w.copyIntro],
    ]
    for (const template of TEMPLATES) {
      const entry = w.entries[template.id]
      texts.push([`${template.id}.name`, entry.name], [`${template.id}.summary`, entry.summary])
      texts.push([`${template.id}.hint`, entry.hint])
      if (entry.warning) texts.push([`${template.id}.warning`, entry.warning])
      entry.items.forEach((item, position) => {
        for (const [key, text] of Object.entries(item)) {
          texts.push([`${template.id}.items.${position}.${key}`, text])
        }
      })
    }
    return texts
  }

  for (const code of ['en', 'es'] as const) {
    test(`in ${code} reads as description, with no clause, promise or advice in it`, () => {
      for (const [key, text] of templateTexts(code)) {
        // The warning is a statement about the law; it may say "contract".
        expect(text, key).not.toMatch(CLAUSE_LIKE[code])
        for (const sentence of text.split(/(?<=[.!?])\s+/)) {
          expect(sentence.replace(/^(e\.g\.|p\. ej\.)\s+/i, ''), `${key}: ${sentence}`).not.toMatch(
            IMPERATIVE[code],
          )
        }
      }
    })
  }

  test('never mentions interest, in any language, anywhere in the starting points', () => {
    for (const code of ['en', 'es'] as const) {
      const all = JSON.stringify(wordingFor(code).templates)
      expect(all).not.toMatch(/interest|inter[eé]s|\brate\b|\btasa\b/i)
    }
  })
})

describe('applying a template', () => {
  test('puts the items in place, in grey, with nothing a person could send', () => {
    for (const template of TEMPLATES) {
      const draft = applyTemplate(template, words('en', template), 'Dana', newId)
      expect(draft.partyA).toBe('Dana')
      expect(draft.partyB).toBe('')
      expect(draft.terms).toBe('')
      expect(draft.note).toBe('')
      expect(draft.contributions).toHaveLength(template.items.length)
      draft.contributions.forEach((item, position) => {
        const declared = template.items[position]
        expect(item.from).toBe(declared.from === 'you' ? 'A' : 'B')
        expect(item.type).toBe(declared.type)
        expect(item.required).toBe(declared.required)
        expect(item.description).toBe('')
        expect(item.criteria).toBe('')
        expect(item.amount).toBe('')
        expect(item.quantity).toBe('')
        expect(item.unit).toBe('')
        expect(item.example?.description).toBe(words('en', template).items[position].description)
        expect(isExampleOnly(item)).toBe(true)
        if (declared.due === 'SIGNED') expect(item.due).toEqual({ kind: 'ON_AGREEMENT' })
        if (declared.due === 'DATE') expect(item.due).toEqual({ kind: 'DATE', date: '' })
        if (typeof declared.due === 'object') {
          expect(item.due).toEqual({
            kind: 'AFTER_CONTRIBUTION',
            contribution: draft.contributions[declared.due.after].id,
          })
        }
      })
    }
  })

  test('is stopped by the composer’s own check until the person writes something', () => {
    for (const template of TEMPLATES) {
      const draft = applyTemplate(template, words('es', template), 'Dana', newId)
      draft.partyB = 'Sam'
      const built = buildTerms(draft, 2)
      expect(built.ok).toBe(false)
      if (!built.ok) {
        const missing = built.problems.filter((problem) => problem.code === 'DESCRIPTION_MISSING')
        expect(missing).toHaveLength(template.items.length)
      }
    }
  })

  test('sends only what the person typed, never an example', () => {
    const template = templateById('job-deposit-balance')!
    const draft = applyTemplate(template, words('en', template), 'Dana', newId)
    draft.partyB = 'Sam'
    draft.contributions[0].description = 'Deposit'
    draft.contributions[0].amount = '100'
    draft.contributions[1].description = 'Fence repair'
    draft.contributions[1].due = { kind: 'DATE', date: '2026-11-01' }
    draft.contributions[2].description = 'Balance'
    draft.contributions[2].amount = '300'
    const built = buildTerms(draft, 2)
    expect(built.ok).toBe(true)
    if (built.ok) {
      expect(built.terms.contributions.map((item) => item.completion_criteria)).toEqual([
        null,
        null,
        null,
      ])
      expect(JSON.stringify(built.terms)).not.toMatch(/e\.g\./)
    }
  })

  test('keeps its examples when the working copy is stored and read back, and drops them from nothing else', () => {
    const template = templateById('pet-sitting-childcare')!
    const draft = applyTemplate(template, words('en', template), 'Rosa', newId)
    const stored = readDraft(JSON.parse(JSON.stringify(draft)))
    expect(stored).toEqual(draft)
    expect(stored?.contributions[0].example).toEqual({
      description: 'e.g. Walk and feed Biscuit, twice a day',
      criteria: 'e.g. Biscuit fed, walked and the key back',
      quantity: 'e.g. 5',
      unit: 'e.g. days',
    })
    // Nothing of the template’s identity is in the working copy.
    expect(JSON.stringify(draft)).not.toContain(template.id)
  })

  test('is turned round by swapping sides, and nothing else changes', () => {
    const template = templateById('selling-something')!
    const draft = applyTemplate(template, words('en', template), 'Kai', newId)
    const swapped = swapSides(draft)
    expect(swapped.contributions.map((item) => item.from)).toEqual(['B', 'A'])
    expect(swapSides(swapped)).toEqual(draft)
    expect(swapped.contributions[0]).toEqual({ ...draft.contributions[0], from: 'B' })
  })
})

describe('copying a previous yup', () => {
  const original = {
    party_a_name: 'Dana Reyes',
    party_b_name: 'Sam Okafor',
    terms: 'Repair the back fence.',
    contributions: [
      {
        id: 'old-1',
        from: 'B' as const,
        type: 'MONEY' as const,
        description: 'Deposit',
        quantity: null,
        due: { kind: 'ON_AGREEMENT' as const },
        completion_criteria: null,
        required: true,
        amount_minor: 10000,
      },
      {
        id: 'old-2',
        from: 'A' as const,
        type: 'SERVICE' as const,
        description: 'Fence repair',
        quantity: { amount: '2', unit: 'posts' },
        due: { kind: 'DATE' as const, date: '2026-10-16' },
        completion_criteria: 'Both posts are solid',
        required: true,
        amount_minor: null,
      },
      {
        id: 'old-3',
        from: 'B' as const,
        type: 'MONEY' as const,
        description: 'Balance',
        quantity: null,
        due: { kind: 'AFTER_CONTRIBUTION' as const, contribution: 'old-2' },
        completion_criteria: null,
        required: false,
        amount_minor: 30050,
      },
    ],
  }

  test('copies items and terms, clears dates, and points links at the new copies', () => {
    const draft = draftFromCopy(original, 'A', false, 2, newId)
    expect(draft.base).toBeNull()
    expect(draft.partyA).toBe('Dana Reyes')
    expect(draft.partyB).toBe('')
    expect(draft.terms).toBe('Repair the back fence.')
    expect(draft.note).toBe('')
    const [deposit, repair, balance] = draft.contributions
    expect(new Set(draft.contributions.map((item) => item.id)).size).toBe(3)
    expect(draft.contributions.map((item) => item.id)).not.toContain('old-1')
    expect(deposit).toMatchObject({
      from: 'B',
      type: 'MONEY',
      description: 'Deposit',
      amount: '100.00',
      due: { kind: 'ON_AGREEMENT' },
    })
    expect(repair).toMatchObject({
      from: 'A',
      description: 'Fence repair',
      quantity: '2',
      unit: 'posts',
      criteria: 'Both posts are solid',
      due: { kind: 'DATE', date: '' },
    })
    expect(balance).toMatchObject({
      required: false,
      amount: '300.50',
      due: { kind: 'AFTER_CONTRIBUTION', contribution: repair.id },
    })
    // An item is not an example: what was copied is the person’s own text.
    expect(draft.contributions.every((item) => item.example === undefined)).toBe(true)
  })

  test('asks about the other party: kept as written for the same person, otherwise cleared', () => {
    expect(draftFromCopy(original, 'A', true, 2, newId).partyB).toBe('Sam Okafor')
    expect(draftFromCopy(original, 'A', false, 2, newId).partyB).toBe('')
  })

  test('turns a yup written from the second party’s side round, so the author is first', () => {
    const draft = draftFromCopy(original, 'B', true, 2, newId)
    expect(draft.partyA).toBe('Sam Okafor')
    expect(draft.partyB).toBe('Dana Reyes')
    expect(draft.contributions.map((item) => item.from)).toEqual(['A', 'B', 'A'])
    expect(draftFromCopy(original, 'B', false, 2, newId).partyB).toBe('')
  })

  test('is a draft that stops at its missing dates and nothing else', () => {
    const draft = draftFromCopy(original, 'A', true, 2, newId)
    const built = buildTerms(draft, 2)
    expect(built.ok).toBe(false)
    if (!built.ok) expect(built.problems.map((problem) => problem.code)).toEqual(['DATE_MISSING'])
  })
})
