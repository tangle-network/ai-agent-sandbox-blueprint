'use strict'

const queues = new Map()

function openclawConcurrencyKey(spec) {
  if (!spec || spec.command !== 'openclaw' || !Array.isArray(spec.args)) return null

  return spec.env?.OPENCLAW_STATE_DIR || spec.env?.HOME || '/home/agent/.openclaw'
}

function runWithKeyedSerialization(key, task) {
  if (!key) return task()

  const previous = queues.get(key) || Promise.resolve()
  const current = previous.then(task)
  const settled = current.then(() => undefined, () => undefined)
  queues.set(key, settled)

  return current.finally(() => {
    if (queues.get(key) === settled) queues.delete(key)
  })
}

module.exports = { openclawConcurrencyKey, runWithKeyedSerialization }
