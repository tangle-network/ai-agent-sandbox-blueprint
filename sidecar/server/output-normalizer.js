'use strict'

const FINAL_TYPES = new Set([
  'complete',
  'completion',
  'done',
  'final',
  'result',
  'success',
])

const NON_TEXT_TYPES = new Set([
  'error',
  'function',
  'analysis',
  'analysis_delta',
  'reasoning_delta',
  'reasoning',
  'system',
  'thinking',
  'thinking_delta',
  'thinking_end',
  'thinking_start',
  'tool',
  'tool_call',
  'tool_call_delta',
  'tool_result',
  'toolcall',
  'toolcall_delta',
  'user',
])

/**
 * Convert provider envelopes into the text returned by the sidecar contract.
 * The optional CLIs emit JSON objects, JSONL events, or plain text depending
 * on their version, so parsing stays at this boundary rather than in callers.
 */
function normalizeHarnessOutput(harness, rawOutput) {
  const raw = String(rawOutput || '').trim()
  if (!raw) return ''

  const documents = parseJsonDocuments(raw)
  if (!documents.length) return raw

  const candidates = []
  for (const document of documents) {
    collectCandidates(document, candidates, {
      harness,
      assistant: false,
      type: '',
    })
  }

  return selectCandidateText(candidates)
}

function parseJsonDocuments(raw) {
  try {
    return [JSON.parse(raw)]
  } catch {
    const documents = []
    for (const line of raw.split(/\r?\n/)) {
      const candidate = line.trim()
      if (!candidate) continue
      try {
        documents.push(JSON.parse(candidate))
      } catch {
        // Stream-json providers may print a diagnostic line beside JSON.
      }
    }
    return documents
  }
}

function collectCandidates(value, candidates, context) {
  if (typeof value === 'string') {
    if (context.assistant || context.final) {
      addCandidate(candidates, value, context.rank || (context.final ? 100 : 80), context.join)
    }
    return
  }
  if (Array.isArray(value)) {
    for (const item of value) collectCandidates(item, candidates, context)
    return
  }
  if (!value || typeof value !== 'object') return

  const type = normalizedType(value.type || value.event || value.subtype || context.type)
  const role = normalizedType(value.role || value.author?.role || value.message?.role)
  if ((role && !isAssistantRole(role)) || isToolType(type)) return
  const assistant = context.assistant || role === 'assistant' || role === 'model' || isAssistantType(type)
  const final = context.final || value.final === true || FINAL_TYPES.has(type)
  const base = { ...context, assistant, final, type }

  if (Array.isArray(value.payloads)) {
    collectCandidates(value.payloads, candidates, {
      ...base,
      assistant: true,
      final: true,
      rank: 90,
      join: 'line',
    })
  }

  for (const [key, rank] of [
    ['finalText', 100],
    ['final_text', 100],
    ['response', final ? 100 : 85],
    ['answer', final ? 100 : 85],
    ['result', final ? 100 : 90],
    ['output', final ? 95 : 85],
    ['summary', final ? 95 : 75],
  ]) {
    if (value[key] === undefined || value[key] === null) continue
    collectCandidates(value[key], candidates, {
      ...base,
      assistant: true,
      final: true,
      rank,
      join: 'line',
    })
  }

  if (value.delta !== undefined && value.delta !== null && isTextType(type)) {
    collectCandidates(value.delta, candidates, {
      ...base,
      assistant: true,
      rank: base.rank || 70,
      join: 'delta',
    })
  }

  if (value.text !== undefined && value.text !== null && isTextType(type, assistant)) {
    collectCandidates(value.text, candidates, {
      ...base,
      assistant,
      rank: base.rank || (assistant ? 80 : 70),
      join: 'line',
    })
  }

  for (const key of [
    'content',
    'message',
    'assistantMessage',
    'assistantMessageEvent',
    'messages',
    'parts',
    'data',
    'events',
  ]) {
    if (value[key] === undefined || value[key] === null) continue
    const childRole = normalizedType(value[key]?.role)
    if (childRole && !isAssistantRole(childRole)) continue
    collectCandidates(value[key], candidates, {
      ...base,
      assistant: assistant || childRole === 'assistant' || childRole === 'model',
      rank: base.rank || (assistant ? 80 : 0),
      join: base.join || 'line',
    })
  }
}

function addCandidate(candidates, value, rank, join) {
  const text = join === 'delta' ? String(value) : String(value).trim()
  if (!text) return
  candidates.push({ text, rank, join: join || 'line' })
}

function selectCandidateText(candidates) {
  if (!candidates.length) return ''
  const highestRank = Math.max(...candidates.map((candidate) => candidate.rank))
  const selected = candidates.filter((candidate) => candidate.rank === highestRank)
  if (selected[0].join === 'delta') {
    return dedupeExact(selected.map((candidate) => candidate.text)).join('').trim()
  }

  const texts = []
  for (const candidate of selected) {
    if (texts.includes(candidate.text)) continue
    const cumulative = texts.findIndex((text) => text.includes(candidate.text))
    if (cumulative !== -1) continue
    const prior = texts.findIndex((text) => candidate.text.includes(text))
    if (prior !== -1) texts.splice(prior, 1)
    texts.push(candidate.text)
  }
  return texts.join('\n\n').trim()
}

function dedupeExact(values) {
  return values.filter((value, index) => values.indexOf(value) === index)
}

function isAssistantRole(role) {
  return role === 'assistant' || role === 'model' || role === 'agent'
}

function isAssistantType(type) {
  return type === 'assistant' || type === 'assistant_message' || type === 'message_update' ||
    type === 'message_end' || type === 'text' || type === 'text_delta'
}

function isToolType(type) {
  return type.startsWith('tool') || type.includes('tool_execution') || type.includes('function_call')
}

function isTextType(type, assistant = true) {
  if (NON_TEXT_TYPES.has(type)) return false
  if (type === 'tool_use' || type === 'tool_result') return false
  return assistant || type === '' || FINAL_TYPES.has(type) || isAssistantType(type)
}

function normalizedType(value) {
  return String(value || '').trim().toLowerCase().replace(/[-\s]/g, '_')
}

module.exports = { normalizeHarnessOutput, parseJsonDocuments }
