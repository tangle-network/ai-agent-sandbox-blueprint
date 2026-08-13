'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const { agents, harnessCommand, selectHarness } = require('./harnesses')

test('registry exposes Prime Agent', () => {
  assert.deepEqual(
    agents.find((agent) => agent.identifier === 'prime'),
    {
      identifier: 'prime',
      displayName: 'Prime Agent',
      description: 'Runs Prime Agent in one-shot print mode.',
    },
  )
})

test('Prime Agent uses the official one-shot print contract', () => {
  const spec = harnessCommand('prime', {
    message: 'review this repository',
    sessionId: 'session/42',
    timeout: 2500,
    backend: {
      provider: 'openai',
      model: 'gpt-5.1',
      thinking: 'high',
    },
  })

  assert.equal(spec.command, 'prime-agent')
  assert.deepEqual(spec.args, [
    '-p',
    '--no-session',
    '--provider',
    'openai',
    '--model',
    'gpt-5.1',
    '--thinking',
    'high',
    'review this repository',
  ])
  assert.equal(spec.timeout, 2500)
  assert.equal(spec.env.PRIME_AGENT_CODING_AGENT_DIR, '/tmp/blueprint-prime-agent-session_42')
  assert.equal(spec.env.PRIME_AGENT_INSTALL_UV, '1')
})

test('Prime Agent preserves prompt boundaries as one argument', () => {
  const message = 'quote "this"; do not run $(rm -rf /);\nsecond line'
  const spec = harnessCommand('prime', { message })
  assert.equal(spec.args.at(-1), message)
  assert.equal(spec.args.filter((arg) => arg === message).length, 1)
})

test('backend type selects Prime Agent from the default identifier', () => {
  assert.equal(selectHarness('default', { type: 'prime' }), 'prime')
})
