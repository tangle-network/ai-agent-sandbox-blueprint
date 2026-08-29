'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const test = require('node:test')
const { agents, harnessCommand, selectHarness } = require('./harnesses')
const {
  advertisedAgents,
  manifest,
  resolveHarnessEntries,
} = require('./harness-manifest')

test('manifest exposes Prime Agent', () => {
  assert.deepEqual(
    manifest.find((agent) => agent.identifier === 'prime'),
    {
      identifier: 'prime',
      capabilityId: 'prime',
      displayName: 'Prime Agent',
      description: 'Runs Prime Agent in one-shot print mode.',
      command: 'prime-agent',
    },
  )
})

test('Prime Agent keeps persistent config and disables saved sessions', () => {
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
  assert.equal(spec.env.PRIME_AGENT_CODING_AGENT_DIR, '/home/agent/.prime/agent')
  assert.equal(spec.env.PRIME_AGENT_SESSION_DIR, undefined)
  assert.equal(spec.env.PRIME_AGENT_INSTALL_UV, '1')
})

test('Prime Agent preserves prompt boundaries as one argument', () => {
  const message = 'quote "this"; do not run $(rm -rf /);\nsecond line'
  const spec = harnessCommand('prime', { message })
  assert.equal(spec.args.at(-1), message)
  assert.equal(spec.args.filter((arg) => arg === message).length, 1)
})

test('backend type selects Prime Agent from the default identifier', () => {
  assert.equal(selectHarness('default', { type: 'prime' }, [
    { identifier: 'default' },
    { identifier: 'prime' },
  ]), 'prime')
})

test('backend type cannot bypass the advertised harness subset', () => {
  assert.equal(selectHarness('default', { type: 'prime' }, [
    { identifier: 'default' },
    { identifier: 'codex' },
  ]), null)
})

test('default selection falls back only to an advertised harness', () => {
  assert.equal(selectHarness('default', {}, [
    { identifier: 'default' },
    { identifier: 'hermes' },
  ]), 'hermes')
})

test('Z.AI credentials select the OpenCode harness', () => {
  assert.equal(selectHarness('default', {}, [
    { identifier: 'default' },
    { identifier: 'opencode' },
  ], { ZAI_API_KEY: 'test-key' }), 'opencode')
})

test('manifest has one entry for each supported CLI', () => {
  const identifiers = manifest.map((agent) => agent.identifier)
  assert.equal(manifest.length, 14)
  assert.equal(new Set(identifiers).size, identifiers.length)
})

test('install and verification scripts cover every manifest entry', () => {
  const sidecarRoot = path.resolve(__dirname, '..')
  const install = fs.readFileSync(path.join(sidecarRoot, 'scripts/install-harness.sh'), 'utf8')
  const verify = fs.readFileSync(path.join(sidecarRoot, 'scripts/verify-harnesses.sh'), 'utf8')
  for (const entry of manifest) {
    assert.match(install, new RegExp(`\\b${entry.identifier}\\b`), `missing installer entry ${entry.identifier}`)
    assert.match(verify, new RegExp(`\\b${entry.command}\\b`), `missing verification entry ${entry.command}`)
  }
})

test('all build mode advertises all manifest entries', () => {
  assert.deepEqual(
    advertisedAgents({ env: { SIDECAR_HARNESSES: 'all' }, commandExists: () => false })
      .map((agent) => agent.identifier),
    ['default', ...manifest.map((agent) => agent.identifier)],
  )
})

test('subset mode filters the registry to configured installed CLIs', () => {
  const entries = resolveHarnessEntries({
    env: { SIDECAR_HARNESSES: 'codex, prime, missing' },
    commandExists: (command) => command === 'codex',
  })
  assert.deepEqual(entries.map((entry) => entry.identifier), ['codex'])
})

test('auto mode advertises only binaries found on PATH', () => {
  const entries = resolveHarnessEntries({
    env: { PATH: '/fake/bin' },
    commandExists: (command) => command === 'hermes',
  })
  assert.deepEqual(entries.map((entry) => entry.identifier), ['hermes'])
})

test('runtime registry always keeps the default auto-select entry', () => {
  assert.equal(agents[0].identifier, 'default')
})
