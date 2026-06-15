import { useMemo, useState } from 'react';
import { Link } from 'react-router';
import { useAccount } from 'wagmi';
import { cn } from '@tangle-network/blueprint-ui';
import { Button } from '@tangle-network/blueprint-ui/components';
import {
  ConsoleChip,
  ConsolePage,
  ConsoleSection,
} from '~/components/console/ConsolePrimitives';
import { ConnectWalletPanel } from '~/components/shared/ConnectWalletPanel';
import { CopyButton } from '~/components/shared/CopyButton';
import { OperatorIdentity } from '~/components/shared/VisualIdentity';
import {
  SANDBOX_ONCHAIN_BLUEPRINT_ID,
  INSTANCE_ONCHAIN_BLUEPRINT_ID,
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
} from '~/lib/config';
import {
  coerceEndpoint,
  useOperatorReadiness,
  type ProbeResult,
} from '~/lib/hooks/useOperatorReadiness';

/**
 * Operator onboarding — the operator side of the two-sided marketplace.
 *
 * Single screen. Each block discloses the next: connect, pick mode(s),
 * advertise capacity, set pricing defaults, declare TEE backend, then copy
 * the exact runtime command + env derived from the operator's choices, and
 * confirm the node answers with a live readiness probe.
 *
 * Every runtime value here is sourced from the repo — see `docs/runbook.md`
 * (§1 env table, §2 default ports), `Dockerfile`, and
 * `contracts/src/AgentSandboxBlueprint.sol` (onRegister decodes a uint32
 * advertised capacity from registrationInputs). No invented defaults.
 */

type ModeId = 'sandbox' | 'instance' | 'tee';

interface ModeSpec {
  id: ModeId;
  label: string;
  blueprintId: string;
  binary: string;
  defaultPort: number;
  summary: string;
  capacityScope: 'multi' | 'single';
}

// Binaries, ports and blueprint ids per variant: docs/runbook.md §1–2 and
// the operator-binaries reference (§7). Sandbox = cloud multi-tenant;
// instance/TEE = single-tenant per service.
const MODES: ModeSpec[] = [
  {
    id: 'sandbox',
    label: 'Sandbox',
    blueprintId: SANDBOX_ONCHAIN_BLUEPRINT_ID,
    binary: 'ai-agent-sandbox-blueprint-bin',
    defaultPort: 9100,
    summary: 'Shared cloud capacity. Hosts ephemeral sidecar containers via Docker.',
    capacityScope: 'multi',
  },
  {
    id: 'instance',
    label: 'Instance',
    blueprintId: INSTANCE_ONCHAIN_BLUEPRINT_ID,
    binary: 'ai-agent-instance-blueprint-bin',
    defaultPort: 9200,
    summary: 'Dedicated single-tenant node. One sandbox per service, lifecycle self-reported.',
    capacityScope: 'single',
  },
  {
    id: 'tee',
    label: 'TEE Instance',
    blueprintId: TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
    binary: 'ai-agent-tee-instance-blueprint-bin',
    defaultPort: 9300,
    summary: 'Confidential variant. Brings up sandboxes inside a TEE with attestation.',
    capacityScope: 'single',
  },
];

// TEE_BACKEND accepted values: docs/runbook.md §1 "TEE instance only".
const TEE_BACKENDS: Array<{ id: string; label: string; needsKey?: string }> = [
  { id: 'phala', label: 'Phala dstack', needsKey: 'PHALA_API_KEY' },
  { id: 'nitro', label: 'AWS Nitro Enclaves' },
  { id: 'gcp', label: 'GCP Confidential Space' },
  { id: 'azure', label: 'Azure SKR' },
  { id: 'direct', label: 'Direct hardware (TDX / SEV)' },
];

// Default sidecar image: docs/runbook.md §1 "Sandbox-mode only".
const DEFAULT_SIDECAR_IMAGE = 'blueprint-sidecar:all-harness';

function ModeCard({
  spec,
  selected,
  onToggle,
}: {
  spec: ModeSpec;
  selected: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-pressed={selected}
      className={cn(
        'flex flex-col items-start gap-2 rounded-[5px] border p-4 text-left transition-[background-color,border-color,box-shadow]',
        selected
          ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)]',
      )}
    >
      <div className="flex w-full items-center justify-between gap-2">
        <span className="font-display text-base font-bold text-[var(--sandbox-console-text)]">{spec.label}</span>
        <span
          className={cn(
            'flex h-5 w-5 items-center justify-center rounded-[4px] border',
            selected
              ? 'border-[var(--sandbox-console-brand)] bg-[var(--sandbox-console-brand)] text-white'
              : 'border-[var(--sandbox-console-border)] text-transparent',
          )}
        >
          <span className="i-ph:check-bold text-xs" />
        </span>
      </div>
      <p className="text-sm leading-5 text-[var(--sandbox-console-muted)]">{spec.summary}</p>
      <span className="font-data text-[11px] text-[var(--sandbox-console-subtle)]">
        #{spec.blueprintId} · :{spec.defaultPort}
      </span>
    </button>
  );
}

function FieldLabel({ children, hint }: { children: string; hint?: string }) {
  return (
    <div className="mb-1.5 flex items-baseline justify-between gap-3">
      <label className="font-data text-[11px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
        {children}
      </label>
      {hint ? <span className="font-data text-[11px] text-[var(--sandbox-console-subtle)]">{hint}</span> : null}
    </div>
  );
}

const inputClass =
  'h-10 w-full rounded-[4px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3 font-data text-sm text-[var(--sandbox-console-text)] outline-none transition-colors placeholder:text-[var(--sandbox-console-subtle)] focus:border-[var(--sandbox-console-brand-border)] focus:bg-[var(--sandbox-console-control-hover)]';

function probeTone(result: ProbeResult<unknown>): 'ready' | 'danger' | 'warn' | 'muted' {
  if (result.state === 'ok') return 'ready';
  if (result.state === 'error') return 'danger';
  if (result.state === 'pending') return 'warn';
  return 'muted';
}

function probeLabel(result: ProbeResult<unknown>): string {
  if (result.state === 'ok') return result.status ? `${result.status} OK` : 'OK';
  if (result.state === 'error') return result.error ?? 'failed';
  if (result.state === 'pending') return 'probing';
  return 'idle';
}

export default function OperatorRegister() {
  const { address, isConnected } = useAccount();
  const [selected, setSelected] = useState<Set<ModeId>>(new Set(['sandbox']));
  const [capacity, setCapacity] = useState('20');
  const [publicHost, setPublicHost] = useState('');
  const [baseRate, setBaseRate] = useState('');
  const [teeBackend, setTeeBackend] = useState(TEE_BACKENDS[0]!.id);
  const [endpoint, setEndpoint] = useState('');
  const [probeActive, setProbeActive] = useState(false);

  const teeSelected = selected.has('tee');
  const selectedSpecs = useMemo(() => MODES.filter((m) => selected.has(m.id)), [selected]);
  // The advertised endpoint's port defaults to the first selected variant's port.
  const primaryPort = selectedSpecs[0]?.defaultPort ?? 9100;

  function toggleMode(id: ModeId) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        if (next.size > 1) next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  const capacityNum = Number.parseInt(capacity, 10);
  const capacityValid = Number.isFinite(capacityNum) && capacityNum > 0 && capacityNum <= 4_294_967_295;
  const advertisesCapacity = selectedSpecs.some((m) => m.capacityScope === 'multi');

  const command = useMemo(() => buildOperatorCommand({
    specs: selectedSpecs,
    capacity: capacityValid ? capacityNum : null,
    publicHost: publicHost.trim(),
    baseRate: baseRate.trim(),
    teeSelected,
    teeBackend,
    advertisesCapacity,
  }), [selectedSpecs, capacityValid, capacityNum, publicHost, baseRate, teeSelected, teeBackend, advertisesCapacity]);

  const readiness = useOperatorReadiness(endpoint, probeActive);
  const r = readiness.data;

  return (
    <ConsolePage
      title="Become an operator"
      eyebrow="Operator onboarding"
      actions={(
        <Link to="/operators">
          <Button variant="secondary" size="sm">
            <span className="i-ph:arrow-left text-base" />
            Directory
          </Button>
        </Link>
      )}
    >
      <div className="mx-auto w-full max-w-3xl space-y-4">
        <p className="max-w-2xl text-sm leading-6 text-[var(--sandbox-console-muted)]">
          Run a node that hosts AI agent sandboxes for the Tangle marketplace. Register on-chain with
          your advertised capacity, run the operator binary, then confirm your node answers below.
        </p>

        {!isConnected ? (
          <ConnectWalletPanel
            title="Connect the wallet you operate with"
            description="Operator registration is an on-chain transaction signed by your operator key. Connect to derive your install command and registration call."
          />
        ) : (
          <div className="sandbox-console-panel flex items-center gap-3 rounded-[5px] p-3.5">
            <span className="i-ph:seal-check text-lg text-[var(--sandbox-console-success)]" />
            <div className="min-w-0">
              <p className="font-data text-[11px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
                Operator address
              </p>
              {address ? <OperatorIdentity address={address} /> : null}
            </div>
          </div>
        )}

        <ConsoleSection title="Deployment mode">
          <div className="grid gap-2 p-3.5 sm:grid-cols-3">
            {MODES.map((spec) => (
              <ModeCard
                key={spec.id}
                spec={spec}
                selected={selected.has(spec.id)}
                onToggle={() => toggleMode(spec.id)}
              />
            ))}
          </div>
        </ConsoleSection>

        <ConsoleSection title="Registration">
          <div className="grid gap-4 p-3.5 sm:grid-cols-2">
            <div className={cn(!advertisesCapacity && 'opacity-50')}>
              <FieldLabel hint={advertisesCapacity ? 'uint32' : 'sandbox mode only'}>
                Advertised capacity
              </FieldLabel>
              <input
                className={inputClass}
                type="number"
                min={1}
                inputMode="numeric"
                value={capacity}
                disabled={!advertisesCapacity}
                onChange={(e) => setCapacity(e.target.value)}
                placeholder="20"
              />
              <p className="mt-1.5 text-[11px] leading-4 text-[var(--sandbox-console-subtle)]">
                {advertisesCapacity
                  ? 'Max concurrent sandboxes. Encoded into the registration call; the contract stores it as operatorMaxCapacity.'
                  : 'Instance and TEE modes are single-tenant — capacity is not advertised at registration.'}
              </p>
              {advertisesCapacity && !capacityValid ? (
                <p className="mt-1 font-data text-[11px] text-[var(--sandbox-console-danger)]">
                  Enter a whole number between 1 and 4,294,967,295.
                </p>
              ) : null}
            </div>

            <div>
              <FieldLabel hint="optional">Public host</FieldLabel>
              <input
                className={inputClass}
                value={publicHost}
                onChange={(e) => setPublicHost(e.target.value)}
                placeholder="node.example.com"
              />
              <p className="mt-1.5 text-[11px] leading-4 text-[var(--sandbox-console-subtle)]">
                Externally-reachable hostname. Required behind NAT/VPN; sets PUBLIC_HOST.
              </p>
            </div>

            <div>
              <FieldLabel hint="optional">Base job rate</FieldLabel>
              <input
                className={inputClass}
                value={baseRate}
                onChange={(e) => setBaseRate(e.target.value)}
                placeholder="leave blank for chain defaults"
              />
              <p className="mt-1.5 text-[11px] leading-4 text-[var(--sandbox-console-subtle)]">
                Drives ConfigureJobRates. Blank uses the on-chain default job rates.
              </p>
            </div>

            {teeSelected ? (
              <div>
                <FieldLabel hint="TEE_BACKEND">TEE backend</FieldLabel>
                <div className="flex flex-wrap gap-1.5">
                  {TEE_BACKENDS.map((backend) => {
                    const active = backend.id === teeBackend;
                    return (
                      <button
                        key={backend.id}
                        type="button"
                        onClick={() => setTeeBackend(backend.id)}
                        className={cn(
                          'inline-flex h-8 items-center rounded-[4px] border px-2.5 font-data text-xs font-semibold transition-colors',
                          active
                            ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)]'
                            : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] hover:text-[var(--sandbox-console-text)]',
                        )}
                      >
                        {backend.label}
                      </button>
                    );
                  })}
                </div>
                <div className="mt-2">
                  <ConsoleChip tone="brand">
                    <span className="i-ph:shield-check mr-1 text-xs" />
                    TEE capability: {teeBackend}
                  </ConsoleChip>
                </div>
              </div>
            ) : null}
          </div>
        </ConsoleSection>

        <ConsoleSection title="Install">
          <div className="space-y-3 p-3.5">
            <div className="flex items-start justify-between gap-3">
              <p className="text-sm leading-5 text-[var(--sandbox-console-muted)]">
                Import an operator key, then run the binary supervised (systemd or equivalent). Set{' '}
                <code className="font-data text-[var(--sandbox-console-secondary)]">SESSION_AUTH_SECRET</code> —
                without it, sessions and at-rest secrets re-key on every restart.
              </p>
              <CopyButton value={command} className="text-base" />
            </div>
            <pre className="max-h-96 overflow-auto rounded-[4px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-bg)] p-3.5 font-data text-[12.5px] leading-relaxed text-[var(--sandbox-console-secondary)]">
              {command}
            </pre>
            {!isConnected ? (
              <p className="font-data text-[11px] text-[var(--sandbox-console-warning)]">
                Connect your wallet to prefill the operator address in the registration call.
              </p>
            ) : null}
          </div>
        </ConsoleSection>

        <ConsoleSection title="Readiness">
          <div className="space-y-3 p-3.5">
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
              <input
                className={inputClass}
                value={endpoint}
                onChange={(e) => {
                  setEndpoint(e.target.value);
                  setProbeActive(false);
                }}
                placeholder={`http://${publicHost.trim() || 'node.example.com'}:${primaryPort}`}
              />
              <Button
                variant="secondary"
                onClick={() => {
                  if (endpoint.trim()) setProbeActive(true);
                }}
                disabled={!endpoint.trim()}
                className="shrink-0"
              >
                <span className={cn('text-base', readiness.isFetching ? 'i-ph:circle-notch animate-spin' : 'i-ph:radar')} />
                {probeActive ? 'Re-check' : 'Check node'}
              </Button>
            </div>
            <p className="text-[11px] leading-4 text-[var(--sandbox-console-subtle)]">
              Probes <code className="font-data">/health</code>, <code className="font-data">/readyz</code> and{' '}
              <code className="font-data">/api/capabilities</code> directly from your browser. Your node must
              be reachable and send CORS headers for these unauthenticated routes.
            </p>

            {probeActive ? (
              <div className="space-y-3">
                <div className="grid gap-2 sm:grid-cols-3">
                  {r ? (
                    <>
                      <ProbeRow path="/readyz" result={r.readyz} />
                      <ProbeRow path="/health" result={r.health} />
                      <ProbeRow path="/api/capabilities" result={r.capabilities} />
                    </>
                  ) : (
                    <div className="sm:col-span-3 flex items-center gap-2 text-sm text-[var(--sandbox-console-muted)]">
                      <span className="i-ph:circle-notch animate-spin" />
                      Contacting {coerceEndpoint(endpoint)}…
                    </div>
                  )}
                </div>

                {r ? <ReadinessVerdict readiness={r} /> : null}
              </div>
            ) : null}
          </div>
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}

function ProbeRow({ path, result }: { path: string; result: ProbeResult<unknown> }) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-[4px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] px-3 py-2.5">
      <span className="truncate font-data text-xs text-[var(--sandbox-console-secondary)]">{path}</span>
      <ConsoleChip tone={probeTone(result)}>{probeLabel(result)}</ConsoleChip>
    </div>
  );
}

function ReadinessVerdict({ readiness }: { readiness: NonNullable<ReturnType<typeof useOperatorReadiness>['data']> }) {
  const backend = readiness.readyz.data?.runtime_backend ?? readiness.health.data?.runtime_backend;
  const runtimeError = readiness.readyz.data?.runtime_error ?? readiness.health.data?.runtime_error;
  const harnesses = readiness.capabilities.data?.harnesses ?? [];
  const capabilities = readiness.capabilities.data?.capabilities ?? [];

  if (readiness.ready) {
    return (
      <div className="rounded-[5px] border border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] p-3.5">
        <div className="flex items-center gap-2">
          <span className="i-ph:check-circle text-lg text-[var(--sandbox-console-success)]" />
          <p className="font-display text-sm font-semibold text-[var(--sandbox-console-text)]">
            Node is ready{backend ? ` · ${backend} backend` : ''}
          </p>
        </div>
        {harnesses.length > 0 ? (
          <div className="mt-3">
            <p className="font-data text-[11px] uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
              Detected harnesses
            </p>
            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {harnesses.map((h) => (
                <ConsoleChip key={h.id} tone="ready">
                  {h.label}
                  {h.mcp ? ' · mcp' : ''}
                </ConsoleChip>
              ))}
            </div>
          </div>
        ) : null}
        {capabilities.length > 0 ? (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {capabilities.map((c) => (
              <ConsoleChip key={c.id} tone="brand">{c.label}</ConsoleChip>
            ))}
          </div>
        ) : null}
      </div>
    );
  }

  if (!readiness.reachable) {
    return (
      <div className="rounded-[5px] border border-red-400/20 bg-red-400/10 p-3.5">
        <div className="flex items-center gap-2">
          <span className="i-ph:plugs text-lg text-[var(--sandbox-console-danger)]" />
          <p className="font-display text-sm font-semibold text-[var(--sandbox-console-text)]">No response from node</p>
        </div>
        <ul className="mt-2 list-disc space-y-1 pl-5 text-[12.5px] leading-5 text-[var(--sandbox-console-muted)]">
          <li>Confirm the binary is running and bound to OPERATOR_API_PORT.</li>
          <li>Confirm the host/port is reachable from the public internet (firewall, NAT, PUBLIC_HOST).</li>
          <li>This page calls your node from the browser — the runtime must return CORS headers for /health, /readyz, /api/capabilities.</li>
        </ul>
      </div>
    );
  }

  // Reachable but not ready: /readyz returned 503.
  return (
    <div className="rounded-[5px] border border-amber-400/20 bg-amber-400/10 p-3.5">
      <div className="flex items-center gap-2">
        <span className="i-ph:warning text-lg text-[var(--sandbox-console-warning)]" />
        <p className="font-display text-sm font-semibold text-[var(--sandbox-console-text)]">
          Node answers but is not ready{backend ? ` · ${backend} backend` : ''}
        </p>
      </div>
      <ul className="mt-2 list-disc space-y-1 pl-5 text-[12.5px] leading-5 text-[var(--sandbox-console-muted)]">
        {readiness.readyz.data?.runtime === false ? (
          <li>Runtime backend unreachable — start Docker (or the configured backend) on the host.</li>
        ) : null}
        {readiness.readyz.data?.store === false ? (
          <li>Persistent store not readable — check BLUEPRINT_STATE_DIR permissions.</li>
        ) : null}
        {runtimeError ? <li className="font-data text-[var(--sandbox-console-danger)]">{runtimeError}</li> : null}
        <li>Once /readyz returns 200 the directory will route work to your node.</li>
      </ul>
    </div>
  );
}

/**
 * Build the copy-pasteable operator install block from the operator's choices.
 * Every var and command is taken from docs/runbook.md and the Dockerfile —
 * placeholders (<...>) mark values only the operator can supply (RPC, keys).
 */
function buildOperatorCommand(opts: {
  specs: ModeSpec[];
  capacity: number | null;
  publicHost: string;
  baseRate: string;
  teeSelected: boolean;
  teeBackend: string;
  advertisesCapacity: boolean;
}): string {
  const { specs, capacity, publicHost, baseRate, teeSelected, teeBackend, advertisesCapacity } = opts;
  const primary = specs[0] ?? MODES[0]!;

  const lines: string[] = [];
  lines.push('# 1. Import your operator key into the keystore');
  lines.push('cargo tangle key import --keystore-uri file:///var/lib/tangle/keystore');
  lines.push('');
  lines.push('# 2. Register on-chain for each selected blueprint');
  lines.push('#    registerOperator(uint64 blueprintId, bytes registrationInputs, string rpcAddress)');
  const rpc = `http://${publicHost || '<PUBLIC_HOST>'}:${primary.defaultPort}`;
  for (const spec of specs) {
    const inputs = spec.capacityScope === 'multi' && capacity != null
      ? `$(cast abi-encode "f(uint32)" ${capacity})` // operatorMaxCapacity, decoded in onRegister
      : '0x';
    lines.push(
      `cast send <TANGLE_PRECOMPILE> "registerOperator(uint64,bytes,string)" \\\n`
      + `  ${spec.blueprintId} ${inputs} "${rpc}" \\\n`
      + `  --rpc-url <HTTP_RPC_ENDPOINT> --private-key <OPERATOR_KEY>`,
    );
  }
  lines.push('');
  lines.push('# 3. Operator environment');

  const env: Array<[string, string]> = [
    ['KEYSTORE_URI', 'file:///var/lib/tangle/keystore'],
    ['HTTP_RPC_ENDPOINT', '<tangle EVM RPC>'],
    ['TANGLE_WS_URL', '<tangle WS endpoint>'],
    ['BLUEPRINT_STATE_DIR', '/var/lib/tangle/blueprint-state'],
    ['SESSION_AUTH_SECRET', '<32+ byte secret>'],
    ['SANDBOX_UI_AUTH_MODE', 'bearer'],
    ['SANDBOX_UI_BEARER_TOKEN', '<ui ingress token>'],
    ['BLUEPRINT_ID', primary.blueprintId],
    ['OPERATOR_API_PORT', String(primary.defaultPort)],
  ];
  if (publicHost) env.push(['PUBLIC_HOST', publicHost]);
  if (specs.some((s) => s.capacityScope === 'multi')) {
    env.push(['SIDECAR_IMAGE', DEFAULT_SIDECAR_IMAGE]);
  }
  if (teeSelected) {
    env.push(['TEE_BACKEND', teeBackend]);
    if (teeBackend === 'phala') env.push(['PHALA_API_KEY', '<phala dstack api key>']);
    if (teeBackend === 'direct') env.push(['TEE_DIRECT_TYPE', '<tdx | sev | nitro>']);
  }
  for (const [k, v] of env) lines.push(`export ${k}="${v}"`);

  if (baseRate) {
    lines.push('');
    lines.push('# 4. Configure pricing (base job rate)');
    lines.push(`forge script contracts/script/ConfigureJobRates.s.sol \\\n  --rpc-url <HTTP_RPC_ENDPOINT> --broadcast  # base rate ${baseRate}`);
  }

  lines.push('');
  lines.push(`# ${baseRate ? '5' : '4'}. Run the operator (supervise under systemd or equivalent)`);
  for (const spec of specs) {
    lines.push(`${spec.binary} run    # ${spec.label} · :${spec.defaultPort}`);
  }

  if (!advertisesCapacity && capacity != null) {
    // Keep the operator informed: capacity is ignored for single-tenant modes.
    lines.push('');
    lines.push('# Note: selected modes are single-tenant; advertised capacity is not used.');
  }

  return lines.join('\n');
}
