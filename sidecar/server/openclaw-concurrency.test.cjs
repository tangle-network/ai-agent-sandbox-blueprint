'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const {
  openclawConcurrencyKey,
  runWithKeyedSerialization,
} = require('./openclaw-concurrency')

function deferred() {
  let resolve
  const promise = new Promise((done) => { resolve = done })
  return { promise, resolve }
}

test('OpenClaw key follows its shared state directory', () => {
  const spec = {
    command: 'openclaw',
    args: ['agent', '--agent', 'research', '--session-id', 'blueprint-retry'],
    env: { OPENCLAW_STATE_DIR: '/run/openclaw/research' },
  }

  assert.equal(openclawConcurrencyKey(spec), '/run/openclaw/research')
})

test('OpenClaw requests serialize per shared state directory', async () => {
  const firstRelease = deferred()
  const differentRelease = deferred()
  const events = []
  let active = 0
  let peakActive = 0

  const run = (key, name, release) => runWithKeyedSerialization(key, async () => {
    active += 1
    peakActive = Math.max(peakActive, active)
    events.push(`${name}:start`)
    await release.promise
    events.push(`${name}:end`)
    active -= 1
    return name
  })

  const first = run('/home/agent/.openclaw', 'first', firstRelease)
  await new Promise((resolve) => setImmediate(resolve))
  const second = run('/home/agent/.openclaw', 'second', firstRelease)
  const different = run('/run/openclaw/isolated', 'different', differentRelease)
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(events, ['first:start', 'different:start'])
  assert.equal(peakActive, 2)

  differentRelease.resolve()
  assert.equal(await different, 'different')
  assert.deepEqual(events, [
    'first:start',
    'different:start',
    'different:end',
  ])

  firstRelease.resolve()
  assert.equal(await first, 'first')
  assert.equal(await second, 'second')
  assert.deepEqual(events, [
    'first:start',
    'different:start',
    'different:end',
    'first:end',
    'second:start',
    'second:end',
  ])
})
