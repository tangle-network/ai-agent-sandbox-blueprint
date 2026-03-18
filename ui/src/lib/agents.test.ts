import { describe, expect, it } from 'vitest';

import {
  BUNDLED_AGENT_OPTIONS,
  BUNDLED_NO_AGENT_VALUE,
  isBundledSandboxImage,
  normalizeAgentIdentifier,
  sanitizeBundledAgentIdentifier,
} from './agents';

describe('agent helpers', () => {
  it('recognizes bundled sandbox images', () => {
    expect(isBundledSandboxImage('agent-dev:latest')).toBe(true);
    expect(isBundledSandboxImage('ghcr.io/tangle-network/sidecar:latest')).toBe(true);
    expect(isBundledSandboxImage('custom/sidecar:dev')).toBe(false);
  });

  it('sanitizes bundled agent identifiers down to the supported allowlist', () => {
    expect(sanitizeBundledAgentIdentifier('default')).toBe('default');
    expect(sanitizeBundledAgentIdentifier(' batch ')).toBe('batch');
    expect(sanitizeBundledAgentIdentifier('a1')).toBe('');
  });

  it('keeps the explicit compute-only option available', () => {
    expect(BUNDLED_AGENT_OPTIONS[0]).toEqual({
      label: 'None (compute only)',
      value: BUNDLED_NO_AGENT_VALUE,
    });
  });

  it('maps the internal none sentinel back to an empty configured identifier', () => {
    expect(sanitizeBundledAgentIdentifier(BUNDLED_NO_AGENT_VALUE)).toBe('');
  });

  it('normalizes identifiers before comparison', () => {
    expect(normalizeAgentIdentifier(' default ')).toBe('default');
    expect(normalizeAgentIdentifier(undefined)).toBe('');
  });
});
