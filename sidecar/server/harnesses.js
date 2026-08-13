'use strict'

const { randomUUID } = require('crypto')
const { hermesCommand } = require('./hermes-command')
const { optionalAgents, optionalHarnessCommand } = require('./optional-harnesses')

const agents = [
  {
    identifier: 'default',
    displayName: 'Default',
    description: 'Uses the first configured local coding harness.',
  },
  {
    identifier: 'codex',
    displayName: 'Codex',
    description: 'Runs the Codex CLI.',
  },
  {
    identifier: 'claude',
    displayName: 'Claude Code',
    description: 'Runs Claude Code.',
  },
  {
    identifier: 'gemini',
    displayName: 'Gemini',
    description: 'Runs Gemini CLI.',
  },
  {
    identifier: 'opencode',
    displayName: 'OpenCode',
    description: 'Runs OpenCode.',
  },
  {
    identifier: 'kimi',
    displayName: 'Kimi',
    description: 'Runs Kimi CLI.',
  },
  {
    identifier: 'prime',
    displayName: 'Prime Agent',
    description: 'Runs Prime Agent in one-shot print mode.',
  },
  {
    identifier: 'hermes',
    displayName: 'Hermes',
    description: 'Runs the Nous Research Hermes Agent CLI.',
  },
  ...optionalAgents,
]

function selectHarness(identifier, backend) {
  const explicitIdentifier = String(identifier || '').trim().toLowerCase()
  const explicitBackend = String(backend?.type || '').trim().toLowerCase()
  const requested = explicitIdentifier && explicitIdentifier !== 'default'
    ? explicitIdentifier
    : explicitBackend
  if (requested) return requested
  if (process.env.SIDECAR_DEFAULT_HARNESS) return process.env.SIDECAR_DEFAULT_HARNESS
  if (process.env.OPENAI_API_KEY) return 'codex'
  if (process.env.ANTHROPIC_API_KEY) return 'claude'
  if (process.env.GEMINI_API_KEY || process.env.GOOGLE_API_KEY) return 'gemini'
  return 'opencode'
}

function primeAgentConfigDir(sessionId) {
  const source = String(sessionId || randomUUID())
  const safe = source.replace(/[^A-Za-z0-9_-]/g, '_').slice(0, 96) || randomUUID()
  return `/tmp/blueprint-prime-agent-${safe}`
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
          PRIME_AGENT_CODING_AGENT_DIR: primeAgentConfigDir(payload.sessionId),
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
