import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import assert from 'node:assert/strict'

import {
  BLUEPRINT_RELEASE_SCHEMA,
  BlueprintReleaseConfigError,
  formatBlueprintReleaseConfig,
  parseTangleBlueprintRegistry,
  readBlueprintReleaseConfig,
  validateBlueprintRegistrations,
  validateBlueprintReleaseConfig,
} from './blueprint-release-config.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const CONFIG_FILE = resolve(ROOT, 'deploy/manifests/base-sepolia/blueprints.json')
const DEPLOYMENT_FILE = resolve(ROOT, 'deploy/manifests/base-sepolia/tnt-core.latest.json')
const SOURCE_PUBLISHER = resolve(ROOT, 'deploy/publish-blueprint-sources.sh')
const CONFIG = JSON.parse(readFileSync(CONFIG_FILE, 'utf8'))
const REGISTRY = [
  'repo\tblueprint_id\tbsm_address\tstatus\tbinary_version_id\tbinary_sha256\tbinary_uri\tbinary_attestation\tnote\ttimestamp',
  'ai-agent-sandbox-blueprint\t10\t0x281d2D1160d80070eBe8989A529b6732C8403625\tregistered\t-\t-\t-\t-\tvariant=sandbox; binary=ai-agent-sandbox-blueprint; no_v0_published\t2026-05-23T00:50:05Z',
  'ai-agent-sandbox-blueprint\t11\t0xDe25dad1757e5Dab5230d44779d7de6ad8181C5C\tregistered\t-\t-\t-\t-\tvariant=instance; binary=ai-agent-instance-blueprint; no_v0_published\t2026-05-23T00:50:07Z',
  'ai-agent-sandbox-blueprint\t12\t0x6D6deBfA88260558597Ad912439Ea1949962b3eb\tregistered\t0\t-\t-\t-\tvariant=tee-instance; binary=ai-agent-tee-instance-blueprint; no_v0_published\t2026-05-23T00:50:09Z',
  'other-blueprint\t13\t0x0000000000000000000000000000000000000002\tregistered\t-\t-\t-\t-\tbinary=other-operator\t2026-05-23T00:50:10Z',
].join('\n')

function invalid(mutator, message) {
  const candidate = structuredClone(CONFIG)
  mutator(candidate)
  assert.throws(() => validateBlueprintReleaseConfig(candidate), BlueprintReleaseConfigError, message)
}

test('the checked-in registry matches the Base Sepolia deployment manifest', () => {
  const config = readBlueprintReleaseConfig(CONFIG_FILE, { deploymentManifest: DEPLOYMENT_FILE })
  assert.equal(config.schema, BLUEPRINT_RELEASE_SCHEMA)
  assert.equal(config.repository, 'tangle-network/ai-agent-sandbox-blueprint')
  assert.equal(config.network, 'base-sepolia')
  assert.equal(config.chainId, '84532')
  assert.equal(config.tangleCore, '0x8299d60f373f3a4a8c4878e335cb9d840e6e3730')
  assert.deepEqual(config.blueprints, [
    {
      binary: 'ai-agent-sandbox-blueprint',
      id: '10',
      bsmAddress: '0x281d2d1160d80070ebe8989a529b6732c8403625',
    },
    {
      binary: 'ai-agent-instance-blueprint',
      id: '11',
      bsmAddress: '0xde25dad1757e5dab5230d44779d7de6ad8181c5c',
    },
    {
      binary: 'ai-agent-tee-instance-blueprint',
      id: '12',
      bsmAddress: '0x6d6debfa88260558597ad912439ea1949962b3eb',
    },
  ])
  assert.equal(Object.isFrozen(config), true)
  assert.equal(Object.isFrozen(config.blueprints), true)
})

test('formats one canonical mapping for shell consumers', () => {
  const config = readBlueprintReleaseConfig(CONFIG_FILE, { deploymentManifest: DEPLOYMENT_FILE })
  assert.equal(
    formatBlueprintReleaseConfig(config, 'entries'),
    'ai-agent-sandbox-blueprint:10\nai-agent-instance-blueprint:11\nai-agent-tee-instance-blueprint:12',
  )
  assert.equal(
    formatBlueprintReleaseConfig(config, 'space'),
    'ai-agent-sandbox-blueprint ai-agent-instance-blueprint ai-agent-tee-instance-blueprint',
  )
})

test('binds configured binaries to exactly one registered tnt-core blueprint', () => {
  const config = readBlueprintReleaseConfig(CONFIG_FILE, { deploymentManifest: DEPLOYMENT_FILE })
  const registrations = parseTangleBlueprintRegistry(REGISTRY)
  assert.deepEqual(
    registrations.slice(0, 3).map(({ binary, id, bsmAddress }) => ({ binary, id, bsmAddress })),
    config.blueprints,
  )
  assert.equal(validateBlueprintRegistrations(config, registrations), config)
})

test('rejects missing, duplicate, inactive, and mismatched tnt-core registrations', () => {
  const config = readBlueprintReleaseConfig(CONFIG_FILE, { deploymentManifest: DEPLOYMENT_FILE })
  const registrations = parseTangleBlueprintRegistry(REGISTRY)
  assert.throws(
    () => validateBlueprintRegistrations(config, registrations.filter(({ binary }) => binary !== config.blueprints[2].binary)),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => validateBlueprintRegistrations(config, [...registrations, registrations[0]]),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => validateBlueprintRegistrations(config, registrations.map((entry) => entry.binary === config.blueprints[1].binary
      ? { ...entry, status: 'pending' }
      : entry)),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => validateBlueprintRegistrations(config, registrations.map((entry) => entry.binary === config.blueprints[1].binary
      ? { ...entry, id: '99' }
      : entry)),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => validateBlueprintRegistrations(config, registrations.map((entry) => entry.binary === config.blueprints[1].binary
      ? { ...entry, bsmAddress: '0x0000000000000000000000000000000000000002' }
      : entry)),
    BlueprintReleaseConfigError,
  )
})

test('CLI validates the deployment binding before emitting entries', () => {
  const result = spawnSync(
    process.execPath,
    [
      resolve(ROOT, 'scripts/blueprint-release-config.mjs'),
      '--file',
      CONFIG_FILE,
      '--deployment-manifest',
      DEPLOYMENT_FILE,
      '--format',
      'entries',
    ],
    { encoding: 'utf8' },
  )
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, 'ai-agent-sandbox-blueprint:10\nai-agent-instance-blueprint:11\nai-agent-tee-instance-blueprint:12\n')
})

test('rejects unknown top-level fields', () => {
  invalid((candidate) => {
    candidate.untrustedOverride = true
  }, 'unknown config fields')
})

test('rejects missing required fields', () => {
  invalid((candidate) => {
    delete candidate.tangleCore
  }, 'missing config fields')
})

test('rejects wrong schema, network, chain, or core', () => {
  invalid((candidate) => {
    candidate.schema = 'tangle-blueprint-release/v0'
  }, 'schema version')
  invalid((candidate) => {
    candidate.repository = 'not-a-repository'
  }, 'repository')
  invalid((candidate) => {
    candidate.network = 'BASE_SEPOLIA'
  }, 'network')
  invalid((candidate) => {
    candidate.chainId = -1
  }, 'chain id')
  invalid((candidate) => {
    candidate.tangleCore = 'not-an-address'
  }, 'tangle core')
})

test('rejects malformed, duplicate, and out-of-range blueprint entries', () => {
  invalid((candidate) => {
    candidate.blueprints[0].binary = '../operator'
  }, 'binary path')
  invalid((candidate) => {
    candidate.blueprints[1].binary = candidate.blueprints[0].binary
  }, 'duplicate binary')
  invalid((candidate) => {
    candidate.blueprints[1].id = candidate.blueprints[0].id
  }, 'duplicate id')
  invalid((candidate) => {
    candidate.blueprints[0].id = '-1'
  }, 'negative id')
  invalid((candidate) => {
    candidate.blueprints[0].id = '18446744073709551616'
  }, 'uint64 overflow')
  invalid((candidate) => {
    candidate.blueprints[0].unexpected = 'value'
  }, 'unknown blueprint fields')
  invalid((candidate) => {
    candidate.blueprints[0].bsmAddress = '0x0000000000000000000000000000000000000000'
  }, 'zero BSM')
})

test('rejects a config that disagrees with explicit deployment expectations', () => {
  assert.throws(
    () => readBlueprintReleaseConfig(CONFIG_FILE, { repository: 'other/repository' }),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => readBlueprintReleaseConfig(CONFIG_FILE, { network: 'ethereum', deploymentManifest: DEPLOYMENT_FILE }),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => readBlueprintReleaseConfig(CONFIG_FILE, { chainId: 1, deploymentManifest: DEPLOYMENT_FILE }),
    BlueprintReleaseConfigError,
  )
  assert.throws(
    () => readBlueprintReleaseConfig(CONFIG_FILE, {
      tangleCore: '0x0000000000000000000000000000000000000001',
      deploymentManifest: DEPLOYMENT_FILE,
    }),
    BlueprintReleaseConfigError,
  )
})

test('rejects a registration from a different repository', () => {
  const config = readBlueprintReleaseConfig(CONFIG_FILE, { deploymentManifest: DEPLOYMENT_FILE })
  const registrations = parseTangleBlueprintRegistry(REGISTRY).map((entry) => entry.binary === config.blueprints[0].binary
    ? { ...entry, repo: 'other-blueprint' }
    : entry)
  assert.throws(() => validateBlueprintRegistrations(config, registrations), BlueprintReleaseConfigError)
})

test('rejects duplicate registry header columns', () => {
  assert.throws(
    () => parseTangleBlueprintRegistry(REGISTRY.replace('repo\tblueprint_id', 'repo\trepo\tblueprint_id')),
    BlueprintReleaseConfigError,
  )
})

test('release paths consume the registry and have no stale TEE variable gate', () => {
  const workflow = readFileSync(resolve(ROOT, '.github/workflows/release.yml'), 'utf8')
  const sourcePublisher = readFileSync(SOURCE_PUBLISHER, 'utf8')
  assert.match(workflow, /blueprint-release-config\.mjs/)
  assert.match(workflow, /verify-blueprint-release-registry\.sh/)
  assert.match(workflow, /KNOWN_RUN_IDS/)
  assert.match(workflow, /multiple new tnt-core runs/)
  assert.doesNotMatch(workflow, /TAG=\"\$\{\{/)
  assert.doesNotMatch(workflow, /TEE_INSTANCE_BLUEPRINT_ID/)
  assert.doesNotMatch(workflow, /PUBLISHES="/)
  assert.match(sourcePublisher, /blueprint-release-config\.mjs/)
  assert.match(sourcePublisher, /verify-blueprint-release-registry\.sh/)
  assert.doesNotMatch(sourcePublisher, /TEE_INSTANCE_BLUEPRINT_ID/)
  assert.doesNotMatch(sourcePublisher, /declare -A BLUEPRINT_IDS/)
})

test('source publisher rejects an unknown selection before any RPC call', () => {
  const result = spawnSync('bash', [SOURCE_PUBLISHER, 'v0.1.4'], {
    env: { ...process.env, ONLY: 'not-in-the-registry' },
    encoding: 'utf8',
  })
  assert.notEqual(result.status, 0)
  assert.match(`${result.stdout}${result.stderr}`, /ONLY=not-in-the-registry is not present/)
})

test('source publisher rejects an unapproved contract override', () => {
  const result = spawnSync('bash', [SOURCE_PUBLISHER, 'v0.1.4'], {
    env: {
      ...process.env,
      ONLY: 'ai-agent-tee-instance-blueprint',
      TANGLE_CORE: '0x0000000000000000000000000000000000000001',
    },
    encoding: 'utf8',
  })
  assert.notEqual(result.status, 0)
  assert.match(`${result.stdout}${result.stderr}`, /expected tangleCore: does not match deployment manifest/)
})

test('source publisher rejects archive bypass before broadcast', () => {
  const result = spawnSync('bash', [SOURCE_PUBLISHER, 'v0.1.4'], {
    env: {
      ...process.env,
      BROADCAST: 'true',
      SKIP_ARCHIVE_VERIFY: '1',
    },
    encoding: 'utf8',
  })
  assert.notEqual(result.status, 0)
  assert.match(`${result.stdout}${result.stderr}`, /SKIP_ARCHIVE_VERIFY=1 is allowed only for dry runs/)
})
