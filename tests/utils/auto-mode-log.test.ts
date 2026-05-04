import { describe, expect, it } from 'vitest'
import type { AutoModeLogRecord } from '~/utils/auto-mode-log'

describe('AutoModeLogRecord type contract', () => {
  it('accepts a valid log record with all fields', () => {
    const record: AutoModeLogRecord = {
      timestamp: '2026-05-03T12:00:00.000Z',
      issueId: 'borabr-lw4.1',
      eventType: 'dispatch_start',
      detail: 'Dispatching: Persistent activity log',
      surface: 'workspace:abc',
      error: null,
    }
    expect(record.timestamp).toBe('2026-05-03T12:00:00.000Z')
    expect(record.eventType).toBe('dispatch_start')
    expect(record.issueId).toBe('borabr-lw4.1')
  })

  it('accepts a record with optional fields absent', () => {
    const record: AutoModeLogRecord = {
      timestamp: '2026-05-03T12:00:00.000Z',
      issueId: '-',
      eventType: 'enabled',
      detail: 'Auto-mode enabled',
    }
    expect(record.surface).toBeUndefined()
    expect(record.error).toBeUndefined()
  })

  it('error field carries failure details', () => {
    const record: AutoModeLogRecord = {
      timestamp: '2026-05-03T12:05:00.000Z',
      issueId: 'borabr-lw4.2',
      eventType: 'dispatch_failed',
      detail: 'Dispatch failed',
      error: 'cmux workspace creation failed: timeout',
    }
    expect(record.error).toContain('timeout')
  })
})

describe('JSONL round-trip contract', () => {
  it('serializes and deserializes a log record through JSON', () => {
    const original: AutoModeLogRecord = {
      timestamp: '2026-05-03T12:00:00.000Z',
      issueId: 'borabr-lw4.1',
      eventType: 'merge_success',
      detail: 'Merged=true closed=true',
      surface: 'workspace:xyz',
      error: null,
    }
    const line = JSON.stringify(original)
    const parsed = JSON.parse(line) as AutoModeLogRecord
    expect(parsed.issueId).toBe(original.issueId)
    expect(parsed.eventType).toBe(original.eventType)
    expect(parsed.detail).toBe(original.detail)
    expect(parsed.surface).toBe(original.surface)
  })

  it('handles multiple lines as independent JSON objects', () => {
    const lines = [
      '{"timestamp":"2026-05-03T12:00:00.000Z","issueId":"a.1","eventType":"dispatch_start","detail":"start"}',
      '{"timestamp":"2026-05-03T12:01:00.000Z","issueId":"a.1","eventType":"dispatch_success","detail":"done"}',
    ]
    const records = lines
      .filter(l => l.trim())
      .map(l => JSON.parse(l) as AutoModeLogRecord)
    expect(records).toHaveLength(2)
    expect(records[0]!.eventType).toBe('dispatch_start')
    expect(records[1]!.eventType).toBe('dispatch_success')
  })

  it('skips malformed lines gracefully', () => {
    const lines = [
      '{"timestamp":"2026-05-03T12:00:00.000Z","issueId":"a.1","eventType":"enabled","detail":"on"}',
      'not-json-at-all',
      '{"timestamp":"2026-05-03T12:01:00.000Z","issueId":"a.2","eventType":"disabled","detail":"off"}',
    ]
    const records = lines
      .filter(l => l.trim())
      .map(l => { try { return JSON.parse(l) as AutoModeLogRecord } catch { return null } })
      .filter((r): r is AutoModeLogRecord => r !== null)
    expect(records).toHaveLength(2)
  })
})
