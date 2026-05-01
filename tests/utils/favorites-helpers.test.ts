import { describe, it, expect } from 'vitest'
import {
  normalizePath,
  deduplicateFavorites,
  sortFavorites,
  isFavorite,
  createFavoriteEntry,
  getProjectWorktreeBadges,
  getProjectWorktreeDisplayName,
  getProjectWorktreeShortPath,
  getVisibleProjectWorktrees,
  type Favorite,
} from '~/utils/favorites-helpers'
import type { ProjectWorktree } from '~/utils/bd-api'

function makeFav(overrides: Partial<Favorite> = {}): Favorite {
  return {
    path: '/home/dev/project',
    name: 'project',
    addedAt: '2025-01-01T00:00:00Z',
    ...overrides,
  }
}

function makeWorktree(overrides: Partial<ProjectWorktree> = {}): ProjectWorktree {
  return {
    rootPath: '/repos/app',
    worktreePath: '/worktrees/github.com/acme/app/feature-a',
    canonicalPath: '/worktrees/github.com/acme/app/feature-a',
    branch: 'feature-a',
    head: 'abc123',
    repoRemote: 'git@github.com:acme/app.git',
    isRoot: false,
    inclusionReason: 'git-worktree-list',
    lastActivityAt: 1_000,
    lastActivitySource: 'file-mtime',
    activityScanLimited: false,
    recentActivityRank: null,
    pullRequest: null,
    prPromoted: false,
    ...overrides,
  }
}

// ---------------------------------------------------------------------------
// normalizePath
// ---------------------------------------------------------------------------
describe('normalizePath', () => {
  it('strips trailing slashes', () => {
    expect(normalizePath('/home/dev/project/')).toBe('/home/dev/project')
  })

  it('strips multiple trailing slashes', () => {
    expect(normalizePath('/home/dev/project///')).toBe('/home/dev/project')
  })

  it('leaves paths without trailing slash unchanged', () => {
    expect(normalizePath('/home/dev/project')).toBe('/home/dev/project')
  })

  it('handles root path', () => {
    expect(normalizePath('/')).toBe('')
  })

  it('handles empty string', () => {
    expect(normalizePath('')).toBe('')
  })
})

// ---------------------------------------------------------------------------
// deduplicateFavorites
// ---------------------------------------------------------------------------
describe('deduplicateFavorites', () => {
  it('returns empty for empty input', () => {
    expect(deduplicateFavorites([])).toEqual([])
  })

  it('keeps unique favorites', () => {
    const favs = [makeFav({ path: '/a' }), makeFav({ path: '/b' })]
    expect(deduplicateFavorites(favs)).toHaveLength(2)
  })

  it('removes duplicates by normalized path', () => {
    const favs = [
      makeFav({ path: '/home/dev/project', name: 'first' }),
      makeFav({ path: '/home/dev/project/', name: 'second' }),
    ]
    const result = deduplicateFavorites(favs)
    expect(result).toHaveLength(1)
    expect(result[0]!.name).toBe('first')
  })

  it('keeps first occurrence of each duplicate', () => {
    const favs = [
      makeFav({ path: '/a', name: 'A1' }),
      makeFav({ path: '/b', name: 'B' }),
      makeFav({ path: '/a', name: 'A2' }),
    ]
    const result = deduplicateFavorites(favs)
    expect(result).toHaveLength(2)
    expect(result[0]!.name).toBe('A1')
  })
})

// ---------------------------------------------------------------------------
// sortFavorites
// ---------------------------------------------------------------------------
describe('sortFavorites', () => {
  const favs = [
    makeFav({ path: '/c', name: 'Charlie' }),
    makeFav({ path: '/a', name: 'Alpha' }),
    makeFav({ path: '/b', name: 'Bravo' }),
  ]

  it('sorts alphabetically ascending', () => {
    const result = sortFavorites(favs, 'alpha')
    expect(result.map(f => f.name)).toEqual(['Alpha', 'Bravo', 'Charlie'])
  })

  it('sorts alphabetically descending', () => {
    const result = sortFavorites(favs, 'alpha-desc')
    expect(result.map(f => f.name)).toEqual(['Charlie', 'Bravo', 'Alpha'])
  })

  it('returns as-is for manual mode', () => {
    const result = sortFavorites(favs, 'manual')
    expect(result.map(f => f.name)).toEqual(['Charlie', 'Alpha', 'Bravo'])
  })

  it('does not mutate the original array', () => {
    const copy = [...favs]
    sortFavorites(favs, 'alpha')
    expect(favs.map(f => f.name)).toEqual(copy.map(f => f.name))
  })

  it('handles empty array', () => {
    expect(sortFavorites([], 'alpha')).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// isFavorite
// ---------------------------------------------------------------------------
describe('isFavorite', () => {
  const favs = [makeFav({ path: '/home/dev/project' }), makeFav({ path: '/other' })]

  it('returns true for existing path', () => {
    expect(isFavorite(favs, '/home/dev/project')).toBe(true)
  })

  it('returns true with trailing slash (normalized)', () => {
    expect(isFavorite(favs, '/home/dev/project/')).toBe(true)
  })

  it('returns false for unknown path', () => {
    expect(isFavorite(favs, '/not/a/favorite')).toBe(false)
  })

  it('returns false for empty list', () => {
    expect(isFavorite([], '/any')).toBe(false)
  })
})

// ---------------------------------------------------------------------------
// createFavoriteEntry
// ---------------------------------------------------------------------------
describe('createFavoriteEntry', () => {
  it('normalizes the path', () => {
    const entry = createFavoriteEntry('/home/dev/project/')
    expect(entry.path).toBe('/home/dev/project')
  })

  it('uses provided name', () => {
    const entry = createFavoriteEntry('/home/dev/project', 'My Project')
    expect(entry.name).toBe('My Project')
  })

  it('extracts folder name when no name provided', () => {
    const entry = createFavoriteEntry('/home/dev/my-app')
    expect(entry.name).toBe('my-app')
  })

  it('sets addedAt to a valid ISO date', () => {
    const entry = createFavoriteEntry('/some/path')
    expect(() => new Date(entry.addedAt)).not.toThrow()
    expect(new Date(entry.addedAt).getFullYear()).toBeGreaterThanOrEqual(2025)
  })
})

describe('project worktree helpers', () => {
  it('uses branch as display name with path fallback', () => {
    expect(getProjectWorktreeDisplayName(makeWorktree({ branch: 'feature/sidebar' }))).toBe('feature/sidebar')
    expect(getProjectWorktreeDisplayName(makeWorktree({ branch: null }))).toBe('feature-a')
  })

  it('shortens long worktree paths', () => {
    expect(getProjectWorktreeShortPath(makeWorktree())).toBe('.../acme/app/feature-a')
  })

  it('returns PR-promoted worktrees before recent-only worktrees', () => {
    const visible = getVisibleProjectWorktrees([
      makeWorktree({ canonicalPath: '/repos/app', isRoot: true, recentActivityRank: 1 }),
      makeWorktree({ canonicalPath: '/worktrees/recent', branch: 'recent', recentActivityRank: 1 }),
      makeWorktree({
        canonicalPath: '/worktrees/open-pr',
        branch: 'open-pr',
        prPromoted: true,
        pullRequest: {
          number: 42,
          title: 'Open PR',
          url: 'https://github.com/acme/app/pull/42',
          state: 'open',
        },
      }),
      makeWorktree({ canonicalPath: '/worktrees/hidden', branch: 'hidden' }),
    ])

    expect(visible.map(worktree => worktree.branch)).toEqual(['open-pr', 'recent'])
  })

  it('builds badges for open PR, merged PR and recent activity', () => {
    expect(getProjectWorktreeBadges(makeWorktree({
      prPromoted: true,
      pullRequest: {
        number: 42,
        title: 'Open PR',
        url: 'https://github.com/acme/app/pull/42',
        state: 'open',
      },
    }))).toContainEqual({ label: 'PR #42 open', tone: 'github-open' })

    const mergedBadges = getProjectWorktreeBadges(makeWorktree({
      prPromoted: true,
      pullRequest: {
        number: 43,
        title: 'Merged PR',
        url: 'https://github.com/acme/app/pull/43',
        state: 'merged',
        mergedAt: '2026-04-30T12:00:00Z',
      },
    }))
    expect(mergedBadges[0]?.label).toContain('PR #43 merged')

    expect(getProjectWorktreeBadges(makeWorktree({ recentActivityRank: 3 }))).toContainEqual({
      label: 'recent #3',
      tone: 'recent',
    })
  })
})
