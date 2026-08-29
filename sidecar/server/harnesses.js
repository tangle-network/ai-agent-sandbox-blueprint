'use strict'

const { hermesCommand } = require('./hermes-command')
const { advertisedAgents } = require('./harness-manifest')
const { optionalHarnessCommand } = require('./optional-harnesses')

// The image advertises only commands present on PATH unless the build sets
// SIDECAR_HARNESSES=all. The default entry remains available for auto-select.
const agents = advertisedAgents()

function selectHarness(identifier, backend, availableAgents = agents, environment = process.env) {
  const explicitIdentifier = String(identifier || '').trim().toLowerCase()
  const explicitBackend = String(backend?.type || '').trim().toLowerCase()
  const requested = explicitIdentifier && explicitIdentifier !== 'default'
    ? explicitIdentifier
    : explicitBackend
  const advertised = new Set(
    availableAgents
      .map((agent) => agent.identifier)
      .filter((identifier) => identifier !== 'default'),
  )
  if (requested) return advertised.has(requested) ? requested : null

  const configuredDefault = String(environment.SIDECAR_DEFAULT_HARNESS || '').trim().toLowerCase()
  if (configuredDefault && advertised.has(configuredDefault)) return configuredDefault
  if (environment.OPENAI_API_KEY && advertised.has('codex')) return 'codex'
  if (environment.ANTHROPIC_API_KEY && advertised.has('claude')) return 'claude'
  if (environment.ZAI_API_KEY && advertised.has('opencode')) return 'opencode'
  if ((environment.GEMINI_API_KEY || environment.GOOGLE_API_KEY) && advertised.has('gemini')) return 'gemini'
  return availableAgents.find((agent) => agent.identifier !== 'default')?.identifier || null
}

function harnessCommand(harness, payload) {
  const message = String(payload.message || '')
  const backend = payload.backend && typeof payload.backend === 'object' ? payload.backend : {}
  const model = backend.model ? String(backend.model) : ''
  const timeout = Number(payload.timeout || 0)

  if (process.env.SIDECAR_AGENT_COMMAND) {
    return {
      command: '/bin/sh',
      args: ['-lc', process.env.SIDECAR_AGENT_COMMAND],
      env: { SIDECAR_AGENT_MESSAGE: message, SIDECAR_AGENT_MODEL: model },
      timeout,
    }
  }

  const optional = optionalHarnessCommand(harness, payload)
  if (optional) return optional

  switch (harness) {
    case 'codex':
      return {
        command: 'codex',
        args: ['exec', '--skip-git-repo-check', '--dangerously-bypass-approvals-and-sandbox', message],
        timeout,
      }
    case 'claude':
      return {
        command: 'claude',
        args: ['-p', message, '--dangerously-skip-permissions'],
        timeout,
      }
    case 'gemini':
      return {
        command: 'gemini',
        args: model
          ? ['--skip-trust', '--yolo', '-m', model, '-p', message]
          : ['--skip-trust', '--yolo', '-p', message],
        timeout,
      }
    case 'kimi':
      return {
        command: 'kimi',
        args: ['-p', message],
        timeout,
      }
    case 'opencode':
      return {
        command: 'opencode',
        args: ['run', message],
        timeout,
      }
    case 'prime': {
      const args = ['-p', '--no-session']
      if (backend.provider) args.push('--provider', String(backend.provider))
      if (model) args.push('--model', model)
      if (backend.thinking) args.push('--thinking', String(backend.thinking))
      args.push(message)
      return {
        command: 'prime-agent',
        args,
        env: {
          PRIME_AGENT_CODING_AGENT_DIR: '/home/agent/.prime/agent',
          PRIME_AGENT_INSTALL_UV: '1',
        },
        timeout,
      }
    }
    case 'hermes':
      return hermesCommand(payload)
    default:
      return null
  }
}

module.exports = { agents, selectHarness, harnessCommand }
