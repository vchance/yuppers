/// <reference types="node" />
import { readFileSync } from 'node:fs'
import { Script, createContext } from 'node:vm'

import { describe, expect, test } from 'vitest'

import { THEME_SCRIPT, themeScriptHash, withThemeScript } from './theme-script.ts'

const policy = readFileSync(
  new URL('../../../backend/src/http/mod.rs', import.meta.url),
  'utf8',
)

test("the service's Content-Security-Policy allows the script by its hash", () => {
  expect(policy).toContain(`script-src 'self' ${themeScriptHash()}`)
})

test('the script goes first in the head', () => {
  const html = withThemeScript('<!doctype html><html><head><meta charset="UTF-8" /></head></html>')
  expect(html).toMatch(/<head>\s*<script>[^<]+<\/script>\s*<meta charset/)
  expect(html).toContain(`<script>${THEME_SCRIPT}</script>`)
})

describe('the script', () => {
  function run(stored: string | null, throws = false) {
    const attributes = new Map<string, string>()
    const context = createContext({
      localStorage: {
        getItem: () => {
          if (throws) throw new Error('blocked')
          return stored
        },
      },
      document: {
        documentElement: { setAttribute: (name: string, value: string) => attributes.set(name, value) },
      },
    })
    new Script(THEME_SCRIPT).runInContext(context)
    return attributes.get('data-theme') ?? null
  }

  test.each(['light', 'dark'])('applies a stored %s', (choice) => {
    expect(run(choice)).toBe(choice)
  })

  test.each([null, 'system', 'sepia', ''])('leaves the system to decide for %s', (stored) => {
    expect(run(stored)).toBeNull()
  })

  test('does nothing when storage is refused', () => {
    expect(run('dark', true)).toBeNull()
  })
})
