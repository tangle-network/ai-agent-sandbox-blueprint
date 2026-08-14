'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const { manifest } = require('./harness-manifest')
const { optionalHarnessCommand } = require('./optional-harnesses')

const builtInHarnesses = new Set(['claude', 'codex', 'opencode', 'kimi', 'gemini', 'prime', 'hermes'])
const optionalAgents = manifest.filter((agent) => !builtInHarnesses.has(agent.identifier))

const sidecarRoot = path.resolve(__dirname, '..')

const payload = {
  message: 'inspect the repository',
  cwd: '/home/agent/workspace',
  timeout: 30,
  backend: { model: 'test-model', provider: 'openai' },
}

test('optional harness identifiers are unique and complete', () => {
  const identifiers = optionalAgents.map((agent) => agent.identifier)
  assert.equal(new Set(identifiers).size, identifiers.length)
  for (const identifier of identifiers) {
    const spec = optionalHarnessCommand(identifier, payload)
    assert.ok(spec, `missing command for ${identifier}`)
    assert.equal(spec.timeout, 30)
  }
})

test('install, verification, and documentation name every optional harness', () => {
  const install = fs.readFileSync(
    path.join(sidecarRoot, 'scripts/install-harness.sh'),
    'utf8',
  )
  const verify = fs.readFileSync(
    path.join(sidecarRoot, 'scripts/verify-harnesses.sh'),
    'utf8',
  )
  const docs = fs.readFileSync(path.join(sidecarRoot, 'README.md'), 'utf8')
  const binaries = {
    amp: 'amp',
    'factory-droids': 'droid',
    pi: 'pi',
    forge: 'forgecode',
    openclaw: 'openclaw',
    qwen: 'qwen',
    copilot: 'copilot',
  }
  for (const agent of optionalAgents) {
    assert.match(install, new RegExp(`\\b${agent.identifier}\\b`))
    assert.match(verify, new RegExp(`\\b${binaries[agent.identifier]}\\b`))
    assert.match(docs, new RegExp(agent.displayName.replace(' ', '\\s+')))
  }
})

test('AMP uses documented execute JSON mode', () => {
  const spec = optionalHarnessCommand('amp', payload)
  assert.deepEqual(spec.args, [
    '--dangerously-allow-all',
    '-x',
    '--stream-json',
    'inspect the repository',
  ])
  assert.equal(spec.env.AMP_MODEL_NAME, 'test-model')
})

test('Factory Droids uses documented non-interactive execute mode', () => {
  const spec = optionalHarnessCommand('factory-droids', payload)
  assert.deepEqual(spec.args, [
    'exec',
    '--skip-permissions-unsafe',
    '--output-format',
    'stream-json',
    '--model',
    'test-model',
    '--cwd',
    '/home/agent/workspace',
    'inspect the repository',
  ])
})

test('Pi uses documented print JSON mode without creating a session', () => {
  const spec = optionalHarnessCommand('pi', payload)
  assert.deepEqual(spec.args, [
    '--print',
    '--mode',
    'json',
    '--no-session',
    '--provider',
    'openai',
    '--model',
    'test-model',
    'inspect the repository',
  ])
})

test('Forge uses documented prompt mode and isolated directory', () => {
  const spec = optionalHarnessCommand('forge', payload)
  assert.deepEqual(spec.args, [
    '-C',
    '/home/agent/workspace',
    '-p',
    'inspect the repository',
  ])
  assert.equal(spec.env.FORGE_SESSION__PROVIDER_ID, 'openai')
  assert.equal(spec.env.FORGE_SESSION__MODEL_ID, 'test-model')
})

test('OpenClaw uses local JSON agent mode', () => {
  const spec = optionalHarnessCommand('openclaw', payload)
  assert.deepEqual(spec.args, [
    'agent',
    '--local',
    '--json',
    '--agent',
    'main',
    '--session-id',
    'blueprint-2276d56b31259ef6e1ed82e6',
    '--model',
    'openai/test-model',
    '--message',
    'inspect the repository',
  ])
  assert.equal(spec.env.OPENCLAW_WORKSPACE_DIR, '/home/agent/workspace')
})

test('Qwen Code uses documented prompt JSON mode', () => {
  const spec = optionalHarnessCommand('qwen', payload)
  assert.deepEqual(spec.args, [
    '--output-format',
    'json',
    '--approval-mode',
    'yolo',
    '--model',
    'test-model',
    '--prompt',
    'inspect the repository',
  ])
})

test('Copilot uses documented silent non-interactive mode', () => {
  const spec = optionalHarnessCommand('copilot', payload)
  assert.deepEqual(spec.args, [
    '--no-auto-update',
    '--allow-all',
    '--silent',
    '--model',
    'test-model',
    '--prompt',
    'inspect the repository',
  ])
})

test('message text remains an argv value, never shell source', () => {
  const message = '$(touch /tmp/pwned); `id`; && exit 9'
  const spec = optionalHarnessCommand('forge', { message })
  assert.equal(spec.args.at(-1), message)
  assert.notEqual(spec.command, '/bin/sh')
})
