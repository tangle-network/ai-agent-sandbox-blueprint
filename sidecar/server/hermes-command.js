'use strict'

/**
 * Build Hermes's documented single-turn command.
 *
 * Quiet mode prints one final response and exits. It does not claim native
 * streaming, cancellation, or session continuation through this sidecar API.
 */
function hermesCommand(payload = {}) {
  const message = String(payload.message || '')
  const provider = payload.backend?.provider ? String(payload.backend.provider) : ''
  const model = payload.backend?.model ? String(payload.backend.model) : ''

  // Sidecar calls cannot answer an approval prompt, so every run is unattended.
  const args = ['chat', '--quiet', '--yolo']
  if (provider) args.push('--provider', provider)
  if (model) args.push('--model', model)
  args.push('-q', message)

  return {
    command: 'hermes',
    args,
    timeout: Number(payload.timeout || payload.timeout_ms || 0),
    env: { HERMES_HOME: '/home/agent/.hermes' },
  }
}

module.exports = { hermesCommand }
