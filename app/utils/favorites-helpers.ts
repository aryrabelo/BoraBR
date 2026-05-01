/**
 * Pure helper functions for project management.
 * Extracted from useProjects composable for testability.
 */
import { getFolderName } from '~/utils/path'
import type { ProjectWorktree } from '~/utils/bd-api'

export interface Project {
  path: string
  name: string
  addedAt: string
}

export type ProjectWorktreeBadgeTone = 'github-open' | 'github-merged' | 'recent' | 'limited'

export interface ProjectWorktreeBadge {
  label: string
  tone: ProjectWorktreeBadgeTone
}

/** @deprecated Use Project instead */
export type Favorite = Project

export type ProjectSortMode = 'alpha' | 'alpha-desc' | 'manual'

/** @deprecated Use ProjectSortMode instead */
export type FavoritesSortMode = ProjectSortMode

/**
 * Normalize path by stripping trailing slashes for consistent comparison.
 */
export function normalizePath(p: string): string {
  return p.replace(/\/+$/, '')
}

/**
 * Deduplicate projects by normalized path, keeping first occurrence.
 */
export function deduplicateProjects(projects: Project[]): Project[] {
  const seen = new Set<string>()
  return projects.filter((proj) => {
    const key = normalizePath(proj.path)
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

/** @deprecated Use deduplicateProjects instead */
export const deduplicateFavorites = deduplicateProjects

/**
 * Sort projects according to the given mode.
 */
export function sortProjects(projects: Project[], mode: ProjectSortMode): Project[] {
  if (mode === 'alpha') {
    return [...projects].sort((a, b) => a.name.localeCompare(b.name))
  }
  if (mode === 'alpha-desc') {
    return [...projects].sort((a, b) => b.name.localeCompare(a.name))
  }
  return projects
}

/** @deprecated Use sortProjects instead */
export const sortFavorites = sortProjects

/**
 * Check if a path is already in the projects list (normalized comparison).
 */
export function isProject(projects: Project[], path: string): boolean {
  const normalized = normalizePath(path)
  return projects.some((f) => normalizePath(f.path) === normalized)
}

/** @deprecated Use isProject instead */
export const isFavorite = isProject

/**
 * Create a new Project entry from a path and optional name.
 */
export function createProjectEntry(path: string, name?: string): Project {
  const normalized = normalizePath(path)
  return {
    path: normalized,
    name: name || getFolderName(path),
    addedAt: new Date().toISOString(),
  }
}

/** @deprecated Use createProjectEntry instead */
export const createFavoriteEntry = createProjectEntry

export function getProjectWorktreeDisplayName(worktree: ProjectWorktree): string {
  return worktree.branch || getFolderName(worktree.canonicalPath || worktree.worktreePath)
}

export function getProjectWorktreeShortPath(worktree: ProjectWorktree): string {
  const path = worktree.canonicalPath || worktree.worktreePath
  const parts = path.split('/').filter(Boolean)
  if (parts.length <= 3) return path
  return `.../${parts.slice(-3).join('/')}`
}

export function isVisibleProjectWorktree(worktree: ProjectWorktree): boolean {
  return !worktree.isRoot && (worktree.prPromoted || !!worktree.recentActivityRank)
}

function projectWorktreeSortGroup(worktree: ProjectWorktree): number {
  if (worktree.prPromoted && worktree.pullRequest?.state === 'open') return 0
  if (worktree.prPromoted) return 1
  return 2
}

export function getVisibleProjectWorktrees(worktrees: ProjectWorktree[]): ProjectWorktree[] {
  return worktrees
    .filter(isVisibleProjectWorktree)
    .sort((a, b) => {
      const groupDelta = projectWorktreeSortGroup(a) - projectWorktreeSortGroup(b)
      if (groupDelta !== 0) return groupDelta

      const rankDelta = (a.recentActivityRank ?? Number.MAX_SAFE_INTEGER)
        - (b.recentActivityRank ?? Number.MAX_SAFE_INTEGER)
      if (rankDelta !== 0) return rankDelta

      return getProjectWorktreeDisplayName(a).localeCompare(getProjectWorktreeDisplayName(b))
    })
}

function formatProjectWorktreeDate(value?: string | number | null): string | null {
  if (!value) return null
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date.toLocaleDateString('pt-BR', { day: '2-digit', month: '2-digit' })
}

export function getProjectWorktreeBadges(worktree: ProjectWorktree): ProjectWorktreeBadge[] {
  const badges: ProjectWorktreeBadge[] = []

  if (worktree.pullRequest?.state === 'open') {
    badges.push({ label: `PR #${worktree.pullRequest.number} open`, tone: 'github-open' })
  } else if (worktree.pullRequest?.state === 'merged') {
    const mergedDate = formatProjectWorktreeDate(worktree.pullRequest.mergedAt)
    badges.push({
      label: mergedDate
        ? `PR #${worktree.pullRequest.number} merged ${mergedDate}`
        : `PR #${worktree.pullRequest.number} merged`,
      tone: 'github-merged',
    })
  }

  if (worktree.recentActivityRank) {
    badges.push({ label: `recent #${worktree.recentActivityRank}`, tone: 'recent' })
  }

  if (worktree.activityScanLimited) {
    badges.push({ label: 'limited', tone: 'limited' })
  }

  return badges
}
