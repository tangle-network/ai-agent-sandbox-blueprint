'use strict'

/**
 * Optional command-line agents with documented non-interactive entrypoints.
 *
 * These specs stay separate from server.js so adding a provider does not grow
 * the HTTP server or turn it into a provider-specific switchboard.
 */
const optionalAgents = [
  {
    identifier: 'amp',
    displayName: 'AMP',
    description: 'Runs Sourcegraph AMP in execute mode.',
  },
  {
    identifier: 'factory-droids',
    displayName: 'Factory Droids',
    description: 'Runs Factory Droids in non-interactive execute mode.',
  },
  {
    identifier: 'pi',
    displayName: 'Pi',
    description: 'Runs Pi in print mode with JSON output.',
  },
  {
    identifier: 'forge',
    displayName: 'Forge',
    description: 'Runs ForgeCode in one-shot prompt mode.',
  },
  {
    identifier: 'openclaw',
    displayName: 'OpenClaw',
    description: 'Runs OpenClaw locally with a JSON result envelope.',
  },
  {
    identifier: 'qwen',
    displayName: 'Qwen Code',
    description: 'Runs Qwen Code in prompt mode.',
  },
  {
    identifier: 'copilot',
    displayName: 'GitHub Copilot CLI',
    description: 'Runs GitHub Copilot CLI in prompt mode.',
  },
]

function optionalHarnessCommand(harness, payload) {
  const message = String(payload.message || '')
  const model = payload.backend?.model ? String(payload.backend.model) : ''
  const provider = payload.backend?.provider ? String(payload.backend.provider) : ''
  const timeout = Number(payload.timeout || 0)
  const env = {}

  switch (harness) {
    case 'amp':
      if (model) env.AMP_MODEL_NAME = model
      return {
        command: 'amp',
        args: ['--dangerously-allow-all', '-x', '--stream-json', message],
        env,
        timeout,
      }
    case 'factory-droids':
      return {
        command: 'droid',
        args: [
          'exec',
          '--skip-permissions-unsafe',
          '--output-format',
          'stream-json',
          ...(model ? ['--model', model] : []),
          '--cwd',
          workspaceFor(payload),
          message,
        ],
        env,
        timeout,
      }
    case 'pi':
      return {
        command: 'pi',
        args: [
          '--print',
          '--mode',
          'json',
          '--no-session',
          ...(provider ? ['--provider', provider] : []),
          ...(model ? ['--model', model] : []),
          message,
        ],
        env,
        timeout,
      }
    case 'forge':
      if (provider) env.FORGE_SESSION__PROVIDER_ID = provider
      if (model) env.FORGE_SESSION__MODEL_ID = model
      return {
        command: 'forgecode',
        args: ['-C', workspaceFor(payload), '-p', message],
        env,
        timeout,
      }
    case 'openclaw':
      return {
        command: 'openclaw',
        args: ['agent', '--local', '--json', '-m', message],
        env,
        timeout,
      }
    case 'qwen':
      return {
        command: 'qwen',
        args: [
          '--output-format',
          'json',
          ...(model ? ['--model', model] : []),
          '--prompt',
          message,
        ],
        env,
        timeout,
      }
    case 'copilot':
      return {
        command: 'copilot',
        args: [
          '--no-auto-update',
          '--allow-all',
          '--silent',
          ...(model ? ['--model', model] : []),
          '--prompt',
          message,
        ],
        env,
        timeout,
      }
    default:
      return null
  }
}

function workspaceFor(payload) {
  return typeof payload.cwd === 'string' && payload.cwd.startsWith('/')
    ? payload.cwd
    : process.env.AGENT_WORKSPACE_ROOT || '/home/agent/workspace'
}

module.exports = { optionalAgents, optionalHarnessCommand }
