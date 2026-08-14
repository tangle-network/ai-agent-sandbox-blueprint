'use strict'

const { createHash } = require('crypto')

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
      // OpenClaw rejects an unscoped local turn. Select a configured agent and
      // isolate the request with a stable session id so retries are replayable.
      const agent = openclawAgentFor(payload)
      return {
        command: 'openclaw',
        args: [
          'agent',
          '--local',
          '--json',
          '--agent',
          agent,
          '--session-id',
          openclawSessionId(payload),
          ...(model ? ['--model', openclawModel(provider, model)] : []),
          '--message',
          message,
        ],
        env: {
          ...env,
          OPENCLAW_WORKSPACE_DIR: workspaceFor(payload),
        },
        timeout,
      }
    case 'qwen':
      return {
        command: 'qwen',
        args: [
          '--output-format',
          'json',
          '--approval-mode',
          'yolo',
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

function openclawAgentFor(payload) {
  const requested = payload.backend?.agent || payload.agent || process.env.OPENCLAW_AGENT_ID || 'main'
  const value = String(requested).trim()
  return value || 'main'
}

function openclawModel(provider, model) {
  if (!provider || model.includes('/')) return model
  return `${provider}/${model}`
}

function openclawSessionId(payload) {
  const requested = payload.sessionId ? String(payload.sessionId).trim() : ''
  if (requested) return `blueprint-${safeSessionPart(requested)}`
  const digest = createHash('sha256')
    .update(JSON.stringify({
      cwd: workspaceFor(payload),
      message: String(payload.message || ''),
      model: String(payload.backend?.model || ''),
      provider: String(payload.backend?.provider || ''),
    }))
    .digest('hex')
    .slice(0, 24)
  return `blueprint-${digest}`
}

function safeSessionPart(value) {
  return value.replace(/[^A-Za-z0-9_-]/g, '_').slice(0, 80) || 'request'
}

function workspaceFor(payload) {
  return typeof payload.cwd === 'string' && payload.cwd.startsWith('/')
    ? payload.cwd
    : process.env.AGENT_WORKSPACE_ROOT || '/home/agent/workspace'
}

module.exports = { optionalHarnessCommand }
