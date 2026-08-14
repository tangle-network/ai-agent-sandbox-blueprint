'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const test = require('node:test')
const { hermesCommand } = require('./hermes-command')

test('Hermes builds a safe quiet single-turn command', () => {
  const prompt = 'review $HOME && echo "keep this as one argument"'
  const spec = hermesCommand({ message: prompt })

  assert.equal(spec.command, 'hermes')
  assert.deepEqual(spec.args, ['chat', '--quiet', '--yolo', '-q', prompt])
  assert.equal(spec.timeout, 0)
  assert.equal(spec.env.HERMES_HOME, '/home/agent/.hermes')
})

test('Hermes routes provider and model without shell interpolation', () => {
  const spec = hermesCommand({
    message: 'solve the task',
    backend: {
      provider: 'openrouter',
      model: 'openrouter/anthropic/claude-sonnet-4',
    },
    timeout_ms: 90000,
  })

  assert.deepEqual(spec.args, [
    'chat',
    '--quiet',
    '--yolo',
    '--provider',
    'openrouter',
    '--model',
    'openrouter/anthropic/claude-sonnet-4',
    '-q',
    'solve the task',
  ])
  assert.equal(spec.timeout, 90000)
})

test('Hermes is wired into the install, verification, image, and docs surfaces', () => {
  const sidecarRoot = path.resolve(__dirname, '..')
  const install = fs.readFileSync(path.join(sidecarRoot, 'scripts/install-harness.sh'), 'utf8')
  const verify = fs.readFileSync(path.join(sidecarRoot, 'scripts/verify-harnesses.sh'), 'utf8')
  const docker = fs.readFileSync(path.join(sidecarRoot, 'Dockerfile.all-harness'), 'utf8')
  const readme = fs.readFileSync(path.join(sidecarRoot, 'README.md'), 'utf8')

  assert.match(install, /install_hermes/)
  assert.match(install, /hermes-agent\.nousresearch\.com\/install\.sh/)
  assert.match(verify, /hermes/)
  assert.match(docker, /Hermes/)
  assert.match(docker, /FROM node@sha256:[0-9a-f]{64}/)
  assert.match(docker, /ENV SIDECAR_HARNESSES="\$\{BLUEPRINT_HARNESSES\}"/)
  assert.match(docker, /\/home\/agent\/.prime\/agent/)
  assert.match(docker, /VOLUME \/home\/agent\/.hermes/)
  assert.doesNotMatch(docker, /VOLUME \/root\/.hermes/)
  assert.match(readme, /Hermes/)
})
