import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const root = process.cwd()

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(resolve(root, path), 'utf8')) as T
}

function readText(path: string): string {
  return readFileSync(resolve(root, path), 'utf8')
}

describe('BoraBR metadata', () => {
  it('uses BoraBR package and repository metadata', () => {
    const pkg = readJson<{
      name: string
      description: string
      repository: { url: string }
      homepage: string
      bugs: { url: string }
    }>('package.json')

    expect(pkg.name).toBe('borabr')
    expect(pkg.description).toContain('BoraBR')
    expect(pkg.repository.url).toBe('https://github.com/aryrabelo/BoraBR.git')
    expect(pkg.homepage).toBe('https://github.com/aryrabelo/BoraBR')
    expect(pkg.bugs.url).toBe('https://github.com/aryrabelo/BoraBR/issues')
  })

  it('uses BoraBR Tauri bundle metadata', () => {
    const tauri = readJson<{
      productName: string
      identifier: string
      app: { windows: Array<{ title: string }> }
    }>('src-tauri/tauri.conf.json')
    const cargo = readText('src-tauri/Cargo.toml')

    expect(tauri.productName).toBe('BoraBR')
    expect(tauri.identifier).toBe('com.aryrabelo.borabr')
    expect(tauri.app.windows[0]?.title).toBe('BoraBR')
    expect(cargo).toContain('name = "borabr"')
    expect(cargo).toContain('description = "BoraBR')
    expect(cargo).toContain('repository = "https://github.com/aryrabelo/BoraBR"')
  })

  it('uses BoraBR in app chrome and about surfaces', () => {
    const files = [
      'app/app.vue',
      'app/pages/index.vue',
      'app/composables/useAppMenu.ts',
      'app/components/layout/AboutDialog.vue',
      'app/components/layout/AppHeader.vue',
      'app/components/layout/UpdateDialog.vue',
      'app/components/dashboard/PrerequisitesCard.vue',
    ]

    for (const file of files) {
      const content = readText(file)
      expect(content, file).toContain('BoraBR')
      expect(content, file).not.toContain('Beads Task-Issue Tracker')
    }
  })
})
