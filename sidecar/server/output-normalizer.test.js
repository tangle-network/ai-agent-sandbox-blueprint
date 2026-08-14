'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const { normalizeHarnessOutput } = require('./output-normalizer')

test('AMP stream JSON returns the final assistant result', () => {
  const output = [
    JSON.stringify({ type: 'system', text: 'starting' }),
    JSON.stringify({
      type: 'assistant',
      message: { role: 'assistant', content: [{ type: 'text', text: 'intermediate' }] },
    }),
    JSON.stringify({ type: 'result', subtype: 'success', result: 'final AMP answer' }),
  ].join('\n')

  assert.equal(normalizeHarnessOutput('amp', output), 'final AMP answer')
})

test('Factory stream JSON extracts assistant content and ignores tools', () => {
  const output = [
    JSON.stringify({ type: 'tool_result', role: 'tool', text: 'secret command output' }),
    JSON.stringify({ type: 'tool_execution_end', result: 'another secret command output' }),
    JSON.stringify({
      type: 'completion',
      message: { role: 'assistant', content: [{ type: 'text', text: 'Factory answer' }] },
    }),
  ].join('\n')

  assert.equal(normalizeHarnessOutput('factory-droids', output), 'Factory answer')
})

test('Pi JSONL joins assistant text deltas', () => {
  const output = [
    JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'thinking_delta', delta: 'private reasoning' } }),
    JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'Pi ' } }),
    JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'answer' } }),
  ].join('\n')

  assert.equal(normalizeHarnessOutput('pi', output), 'Pi answer')
})

test('OpenClaw JSON prefers result payload text over metadata', () => {
  const output = JSON.stringify({
    runId: 'run-1',
    status: 'ok',
    summary: 'metadata summary',
    result: { payloads: [{ text: 'OpenClaw answer' }] },
  })

  assert.equal(normalizeHarnessOutput('openclaw', output), 'OpenClaw answer')
})

test('plain output remains unchanged', () => {
  assert.equal(normalizeHarnessOutput('amp', 'plain assistant output\n'), 'plain assistant output')
})

test('structured output never leaks tool-only JSON as an assistant answer', () => {
  const output = JSON.stringify({
    type: 'tool_result',
    role: 'tool',
    content: 'private command output',
  })

  assert.equal(normalizeHarnessOutput('factory-droids', output), '')
})
