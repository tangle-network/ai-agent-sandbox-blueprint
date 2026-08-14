'use strict'

const fs = require('fs')
const os = require('os')
const path = require('path')

const manifest = Object.freeze(require('./harness-manifest.json').map((entry) => Object.freeze(entry)))
const manifestByIdentifier = new Map(manifest.map((entry) => [entry.identifier, entry]))

function splitHarnessSet(raw) {
  return String(raw || '')
    .split(',')
    .map((identifier) => identifier.trim().toLowerCase())
    .filter(Boolean)
}

function commandExists(command, env = process.env) {
  const pathValue = typeof env.PATH === 'string' && env.PATH
    ? env.PATH
    : ['/usr/local/bin', '/usr/bin', '/bin', os.homedir()].join(path.delimiter)
  for (const directory of pathValue.split(path.delimiter)) {
    if (!directory) continue
    const candidate = path.join(directory, command)
    try {
      const stat = fs.statSync(candidate)
      if (stat.isFile() && (process.platform === 'win32' || (stat.mode & 0o111) !== 0)) {
        return true
      }
    } catch (_error) {
      // A missing binary is the expected result for a subset image.
    }
  }
  return false
}

function resolveHarnessEntries(options = {}) {
  const env = options.env || process.env
  const exists = options.commandExists || ((command) => commandExists(command, env))
  const configured = typeof env.SIDECAR_HARNESSES === 'string'
    ? env.SIDECAR_HARNESSES.trim().toLowerCase()
    : ''

  if (configured === 'all') return manifest

  const requested = configured
    ? new Set(splitHarnessSet(configured))
    : null

  return manifest.filter((entry) => {
    if (requested && !requested.has(entry.identifier)) return false
    return exists(entry.command)
  })
}

function advertisedAgents(options = {}) {
  return [
    {
      identifier: 'default',
      displayName: 'Default',
      description: 'Uses the first configured local coding harness.',
    },
    ...resolveHarnessEntries(options).map(({ identifier, displayName, description }) => ({
      identifier,
      displayName,
      description,
    })),
  ]
}

function capabilityEntries() {
  return manifest.map(({ capabilityId, displayName }) => ({
    id: capabilityId,
    label: displayName,
  }))
}

function manifestEntry(identifier) {
  return manifestByIdentifier.get(identifier) || null
}

module.exports = {
  manifest,
  capabilityEntries,
  advertisedAgents,
  commandExists,
  manifestEntry,
  resolveHarnessEntries,
  splitHarnessSet,
}
