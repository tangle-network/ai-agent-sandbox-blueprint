'use strict'

const path = require('path')

/**
 * Build Hermes's documented single-turn command.
 *
 * Quiet mode prints one final response and exits. It does not claim native
 * streaming, cancellation, or session continuation through this sidecar API.
 */
function hermesCommand(payload = {}) {
  const message = String(payload.message || '')
  const model = payload.backend?.model ? String(payload.backend.model) : ''
  const hermesHome = process.env.HERMES_HOME || path.join(
    process.env.AGENT_WORKSPACE_ROOT || '/home/agent/workspace',
    '.hermes',
  )

  const args = ['chat', '--quiet']
  if (model) args.push('--model', model)
  args.push('-q', message)

  return {
    command: 'hermes',
    args,
    timeout: Number(payload.timeout || payload.timeout_ms || 0),
    env: { HERMES_HOME: hermesHome },
  }
}

module.exports = { hermesCommand }
