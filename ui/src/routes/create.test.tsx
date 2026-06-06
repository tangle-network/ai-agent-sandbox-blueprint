import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import CreatePage from './create';

const {
  currentSearchRef,
  infraStateRef,
  mockNavigate,
  mockUpdateInfra,
  mockValidateService,
  serviceValidationRef,
  mockDeploy,
  mockDeployReset,
} = vi.hoisted(() => ({
  currentSearchRef: { current: '?blueprint=ai-agent-instance-blueprint' },
  infraStateRef: {
    current: {
      blueprintId: '2',
      serviceId: '2',
      serviceValidated: false,
      serviceInfo: null as null | Record<string, unknown>,
    },
  },
  mockNavigate: vi.fn(),
  mockUpdateInfra: vi.fn(),
  mockValidateService: vi.fn(),
  serviceValidationRef: {
    current: {
      serviceInfo: null as null | Record<string, unknown>,
      error: null as string | null,
    },
  },
  mockDeploy: vi.fn(),
  mockDeployReset: vi.fn(),
}));

const SIDECAR_IMAGE_OPTIONS = [
  { label: 'Registry: blueprint-sidecar all-harness', value: 'ghcr.io/tangle-network/blueprint-sidecar:all-harness' },
  { label: 'Local: blueprint-sidecar:all-harness', value: 'blueprint-sidecar:all-harness' },
];

const RUNTIME_BACKEND_OPTIONS = [
  { label: 'Docker', value: 'docker' },
  { label: 'Firecracker', value: 'firecracker' },
];

const PRESET_BLUEPRINTS = {
  sandbox: {
    id: 'ai-agent-sandbox-blueprint',
    name: 'AI Agent Sandbox',
    version: '0.5.0',
    description: 'Sandbox blueprint',
    icon: 'i-ph:cloud',
    color: 'teal',
    jobs: [
      {
        id: 0,
        label: 'Create Sandbox',
        name: 'sandbox_create',
        category: 'lifecycle',
        pricingMultiplier: 50,
        requiresSandbox: false,
        fields: [
          { name: 'name', label: 'Sandbox Name', type: 'text', required: true },
          { name: 'image', label: 'Docker Image', type: 'combobox', defaultValue: 'ghcr.io/tangle-network/blueprint-sidecar:all-harness', options: SIDECAR_IMAGE_OPTIONS },
          { name: 'runtimeBackend', label: 'Runtime Backend', type: 'select', defaultValue: 'docker', options: RUNTIME_BACKEND_OPTIONS },
          { name: 'stack', label: 'Stack', type: 'select', defaultValue: 'default', options: [{ label: 'Default', value: 'default' }] },
          { name: 'agentIdentifier', label: 'Agent Identifier', type: 'text', internal: true },
          { name: 'metadataJson', label: 'Metadata (JSON)', type: 'json', defaultValue: '{}' },
          { name: 'envJson', label: 'Environment Variables (JSON)', type: 'json', defaultValue: '{}' },
          { name: 'sshEnabled', label: 'Enable SSH', type: 'boolean', defaultValue: false },
          { name: 'sshPublicKey', label: 'SSH Public Key', type: 'textarea', defaultValue: '' },
          { name: 'maxLifetimeSeconds', label: 'Max Lifetime (hours)', type: 'number', defaultValue: 86400 },
          { name: 'idleTimeoutSeconds', label: 'Idle Timeout (minutes)', type: 'number', defaultValue: 3600 },
          { name: 'cpuCores', label: 'CPU Cores', type: 'number', defaultValue: 2 },
          { name: 'memoryMb', label: 'Memory (MB)', type: 'number', defaultValue: 2048 },
          { name: 'diskGb', label: 'Disk (GB)', type: 'number', defaultValue: 10 },
          { name: 'teeRequired', label: 'TEE Required', type: 'boolean', defaultValue: false },
          { name: 'teeType', label: 'TEE Type', type: 'select', defaultValue: '0', options: [{ label: 'None', value: '0' }] },
        ],
      },
    ],
    categories: [{ key: 'lifecycle', label: 'Lifecycle' }],
  },
  instance: {
    id: 'ai-agent-instance-blueprint',
    name: 'AI Agent Instance',
    version: '0.4.0',
    description: 'Instance blueprint',
    icon: 'i-ph:cube',
    color: 'blue',
    jobs: [
      {
        id: 0,
        label: 'Provision Instance',
        name: 'instance_provision',
        category: 'lifecycle',
        pricingMultiplier: 50,
        requiresSandbox: false,
        fields: [
          { name: 'name', label: 'Instance Name', type: 'text', required: true },
          { name: 'image', label: 'Docker Image', type: 'combobox', defaultValue: 'ghcr.io/tangle-network/blueprint-sidecar:all-harness', options: SIDECAR_IMAGE_OPTIONS },
          { name: 'runtimeBackend', label: 'Runtime Backend', type: 'select', defaultValue: 'docker', options: RUNTIME_BACKEND_OPTIONS },
          { name: 'stack', label: 'Stack', type: 'select', defaultValue: 'default', options: [{ label: 'Default', value: 'default' }] },
          { name: 'agentIdentifier', label: 'Agent Identifier', type: 'text', internal: true },
          { name: 'metadataJson', label: 'Metadata (JSON)', type: 'json', defaultValue: '{}' },
          { name: 'envJson', label: 'Environment Variables (JSON)', type: 'json', defaultValue: '{}' },
          { name: 'sshEnabled', label: 'Enable SSH', type: 'boolean', defaultValue: false },
          { name: 'sshPublicKey', label: 'SSH Public Key', type: 'textarea', defaultValue: '' },
          { name: 'maxLifetimeSeconds', label: 'Max Lifetime (hours)', type: 'number', defaultValue: 86400 },
          { name: 'idleTimeoutSeconds', label: 'Idle Timeout (minutes)', type: 'number', defaultValue: 3600 },
          { name: 'cpuCores', label: 'CPU Cores', type: 'number', defaultValue: 2 },
          { name: 'memoryMb', label: 'Memory (MB)', type: 'number', defaultValue: 2048 },
          { name: 'diskGb', label: 'Disk (GB)', type: 'number', defaultValue: 10 },
          { name: 'teeRequired', label: 'TEE Required', type: 'boolean', defaultValue: false },
          { name: 'teeType', label: 'TEE Type', type: 'select', defaultValue: '0', options: [{ label: 'None', value: '0' }] },
        ],
      },
      {
        id: 2,
        label: 'Create Workflow',
        name: 'workflow_create',
        category: 'workflow',
        pricingMultiplier: 2,
        requiresSandbox: false,
        fields: [],
      },
    ],
    categories: [{ key: 'lifecycle', label: 'Lifecycle' }],
  },
  teeInstance: {
    id: 'ai-agent-tee-instance-blueprint',
    name: 'AI Agent TEE Instance',
    version: '0.4.0',
    description: 'TEE instance blueprint',
    icon: 'i-ph:shield-check',
    color: 'violet',
    jobs: [
      {
        id: 0,
        label: 'Provision Instance',
        name: 'instance_provision',
        category: 'lifecycle',
        pricingMultiplier: 50,
        requiresSandbox: false,
        fields: [
          { name: 'name', label: 'Instance Name', type: 'text', required: true },
          { name: 'image', label: 'Docker Image', type: 'combobox', defaultValue: 'ghcr.io/tangle-network/blueprint-sidecar:all-harness', options: SIDECAR_IMAGE_OPTIONS },
          { name: 'runtimeBackend', label: 'Runtime Backend', type: 'select', defaultValue: 'docker', options: RUNTIME_BACKEND_OPTIONS },
          { name: 'stack', label: 'Stack', type: 'select', defaultValue: 'default', options: [{ label: 'Default', value: 'default' }] },
          { name: 'agentIdentifier', label: 'Agent Identifier', type: 'text', internal: true },
          { name: 'metadataJson', label: 'Metadata (JSON)', type: 'json', defaultValue: '{}' },
          { name: 'envJson', label: 'Environment Variables (JSON)', type: 'json', defaultValue: '{}' },
          { name: 'sshEnabled', label: 'Enable SSH', type: 'boolean', defaultValue: false },
          { name: 'sshPublicKey', label: 'SSH Public Key', type: 'textarea', defaultValue: '' },
          { name: 'maxLifetimeSeconds', label: 'Max Lifetime (hours)', type: 'number', defaultValue: 86400 },
          { name: 'idleTimeoutSeconds', label: 'Idle Timeout (minutes)', type: 'number', defaultValue: 3600 },
          { name: 'cpuCores', label: 'CPU Cores', type: 'number', defaultValue: 2 },
          { name: 'memoryMb', label: 'Memory (MB)', type: 'number', defaultValue: 2048 },
          { name: 'diskGb', label: 'Disk (GB)', type: 'number', defaultValue: 10 },
          { name: 'teeRequired', label: 'TEE Required', type: 'boolean', defaultValue: false },
          { name: 'teeType', label: 'TEE Type', type: 'select', defaultValue: '0', options: [{ label: 'None', value: '0' }] },
        ],
      },
      {
        id: 2,
        label: 'Create Workflow',
        name: 'workflow_create',
        category: 'workflow',
        pricingMultiplier: 2,
        requiresSandbox: false,
        fields: [],
      },
    ],
    categories: [{ key: 'lifecycle', label: 'Lifecycle' }],
  },
};

function blueprintForSearch(search: string) {
  const params = new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
  const id = params.get('blueprint');
  if (id === PRESET_BLUEPRINTS.sandbox.id) return PRESET_BLUEPRINTS.sandbox;
  if (id === PRESET_BLUEPRINTS.teeInstance.id) return PRESET_BLUEPRINTS.teeInstance;
  return PRESET_BLUEPRINTS.instance;
}

vi.mock('react-router', () => ({
  useNavigate: () => mockNavigate,
  useSearchParams: () => [new URLSearchParams(currentSearchRef.current)],
}));

vi.mock('wagmi', () => ({
  useAccount: () => ({
    address: '0x123400000000000000000000000000000000abcd',
    isConnected: true,
    status: 'connected',
  }),
}));

vi.mock('@nanostores/react', () => ({
  useStore: () => infraStateRef.current,
}));

vi.mock('@tangle-network/blueprint-ui/components', () => ({
  AnimatedPage: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Card: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardContent: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardHeader: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardTitle: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  CardDescription: ({ children, ...props }: { children: React.ReactNode }) => <div {...props}>{children}</div>,
  Button: ({ children, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Badge: ({ children, ...props }: { children: React.ReactNode }) => <span {...props}>{children}</span>,
  Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
  Select: ({
    value,
    onValueChange,
    options,
  }: {
    value?: string;
    onValueChange: (value: string) => void;
    options: Array<{ label: string; value: string }>;
  }) => (
    <select aria-label="Agent" value={value} onChange={(event) => onValueChange(event.target.value)}>
      {options.map((option) => (
        <option key={option.value} value={option.value}>{option.label}</option>
      ))}
    </select>
  ),
  BlueprintJobForm: ({
    job,
    values,
    onChange,
    sections,
  }: {
    job: { fields: Array<Record<string, any>> };
    values: Record<string, unknown>;
    onChange: (name: string, value: unknown) => void;
    sections?: Array<{ fields: string[] }>;
  }) => {
    const fieldNames = sections?.flatMap((section) => section.fields) ?? [];
    const fields = job.fields.filter((field) => fieldNames.includes(field.name) && !field.internal);

    return (
      <div>
        {fields.map((field) => {
          if (field.type === 'select') {
            return (
              <label key={field.name}>
                {field.label}
                <select
                  aria-label={field.label}
                  value={String(values[field.name] ?? field.defaultValue ?? '')}
                  onChange={(event) => onChange(field.name, event.target.value)}
                >
                  {(field.options ?? []).map((option: { label: string; value: string }) => (
                    <option key={option.value} value={option.value}>{option.label}</option>
                  ))}
                </select>
              </label>
            );
          }

          if (field.type === 'boolean') {
            return (
              <label key={field.name}>
                {field.label}
                <input
                  aria-label={field.label}
                  type="checkbox"
                  checked={Boolean(values[field.name])}
                  onChange={(event) => onChange(field.name, event.target.checked)}
                />
              </label>
            );
          }

          return (
            <label key={field.name}>
              {field.label}
              <input
                aria-label={field.label}
                placeholder={field.placeholder}
                value={String(values[field.name] ?? field.defaultValue ?? '')}
                onChange={(event) => onChange(field.name, event.target.value)}
              />
            </label>
          );
        })}
      </div>
    );
  },
  Identicon: () => <div>Identicon</div>,
}));

vi.mock('@tangle-network/blueprint-ui', async () => {
  const React = await vi.importActual<typeof import('react')>('react');

  function initialValues(job: { fields?: Array<Record<string, any>> } | null | undefined) {
    const values: Record<string, unknown> = {};
    for (const field of job?.fields ?? []) {
      if (field.internal) continue;
      values[field.name] = field.defaultValue ?? '';
    }
    return values;
  }

  return {
    infraStore: {},
    updateInfra: mockUpdateInfra,
    useJobForm: (job: { fields?: Array<Record<string, any>> } | null) => {
      const [values, setValues] = React.useState<Record<string, unknown>>(() => initialValues(job));

      React.useEffect(() => {
        setValues(initialValues(job));
      }, [job]);

      return {
        values,
        errors: {},
        onChange: (name: string, value: unknown) => {
          setValues((current) => ({ ...current, [name]: value }));
        },
        validate: () => Boolean(String(values.name || '').trim()),
        reset: () => setValues(initialValues(job)),
      };
    },
    useJobPrice: () => ({
      quote: null,
      isLoading: false,
      formattedPrice: '0 TANGLE',
    }),
    useServiceValidation: () => ({
      validate: mockValidateService,
      isValidating: false,
      serviceInfo: serviceValidationRef.current.serviceInfo,
      error: serviceValidationRef.current.error,
    }),
    formatCost: () => '0 TANGLE',
    getAllBlueprints: () => Object.values(PRESET_BLUEPRINTS),
    getBlueprint: (id: string) => Object.values(PRESET_BLUEPRINTS).find((blueprint) => blueprint.id === id),
    cn: (...values: Array<string | false | null | undefined>) => values.filter(Boolean).join(' '),
  };
});

vi.mock('~/components/shared/InfrastructureModal', () => ({
  InfrastructureModal: ({
    open,
    initialMode,
  }: {
    open: boolean;
    initialMode?: 'existing' | 'new';
  }) => open ? <div data-testid="infra-modal">Infrastructure {initialMode}</div> : null,
  InfraBar: () => <div>Infra Bar</div>,
}));

vi.mock('~/components/shared/JobPriceBadge', () => ({
  JobPriceBadge: () => <span>Price</span>,
}));

vi.mock('~/components/shared/ProvisionProgress', () => ({
  ProvisionProgress: () => <div>Provision Progress</div>,
}));

vi.mock('~/components/shared/InfraSummaryBits', () => ({
  BlueprintBadgeInline: ({ blueprintId }: { blueprintId: string }) => <span>Blueprint {blueprintId}</span>,
}));

vi.mock('~/components/shared/EnvEditor', () => ({
  EnvEditor: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Environment Variables"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

vi.mock('~/lib/hooks/useSandboxReads', () => ({
  useAvailableCapacity: () => ({ data: 5 }),
}));

vi.mock('~/lib/hooks/useCreateDeploy', () => ({
  useCreateDeploy: ({ blueprint }: { blueprint?: { id?: string } }) => {
    const mode = blueprint?.id === 'ai-agent-sandbox-blueprint' ? 'sandbox' : 'instance';
    return {
      mode,
      status: 'idle',
      txHash: undefined,
      error: undefined,
      callId: undefined,
      provision: undefined,
      sandboxDraftKey: undefined,
      operators: [],
      operatorsLoading: false,
      operatorsError: null,
      operatorCount: 0n,
      isNewService: mode === 'instance',
      isInstanceMode: mode === 'instance',
      isTeeInstance: blueprint?.id === 'ai-agent-tee-instance-blueprint',
      hasValidService: mode === 'sandbox',
      contractsDeployed: true,
      canDeploy: true,
      deploy: mockDeploy,
      reset: mockDeployReset,
    };
  },
}));

vi.mock('~/lib/stores/sandboxes', () => ({
  updateSandboxStatus: vi.fn(),
}));

vi.mock('~/lib/stores/instances', () => ({
  updateInstanceStatus: vi.fn(),
}));

function renderSubject(search = '?blueprint=ai-agent-instance-blueprint') {
  currentSearchRef.current = search;
  return render(<CreatePage />);
}

function selectAgentOption(optionName: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Agent' }));
  fireEvent.click(screen.getByRole('option', { name: optionName }));
}

function selectImageOption(optionName: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Docker Image' }));
  fireEvent.click(screen.getByRole('option', { name: optionName }));
}

describe('CreatePage agent configuration', () => {
  beforeEach(() => {
    currentSearchRef.current = '?blueprint=ai-agent-instance-blueprint';
    infraStateRef.current = {
      blueprintId: '2',
      serviceId: '2',
      serviceValidated: false,
      serviceInfo: null,
    };
    mockNavigate.mockReset();
    mockUpdateInfra.mockReset();
    mockValidateService.mockReset();
    serviceValidationRef.current = { serviceInfo: null, error: null };
    mockDeploy.mockReset();
    mockDeployReset.mockReset();
  });

  it('renders the bundled agent selector for instance blueprints', () => {
    renderSubject('?blueprint=ai-agent-instance-blueprint');

    expect(screen.getByText('Agent')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Agent' })).toHaveTextContent('None (compute only)');
    expect(screen.getByText(/Choose an agent already bundled in this image/i)).toBeInTheDocument();
  });

  it('renders the bundled agent selector for TEE instance blueprints', () => {
    renderSubject('?blueprint=ai-agent-tee-instance-blueprint');

    expect(screen.getByText('Agent')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Agent' })).toHaveTextContent('None (compute only)');
  });

  it('shows the selected bundled agent in the review step for instances', () => {
    renderSubject('?blueprint=ai-agent-instance-blueprint');

    fireEvent.change(screen.getByLabelText('Instance Name'), { target: { value: 'Worker One' } });
    selectAgentOption('Batch');
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText('Agent: batch')).toBeInTheDocument();
  });

  it('switches to a free-text agent input for custom instance images', () => {
    renderSubject('?blueprint=ai-agent-instance-blueprint');

    selectImageOption('Custom image...');
    fireEvent.change(screen.getByLabelText('Custom Image'), { target: { value: 'custom/agent-sidecar:1.0.0' } });

    expect(screen.getByPlaceholderText('default')).toBeInTheDocument();
    expect(screen.getByText(/Custom images must already register this agent identifier internally/i)).toBeInTheDocument();
  });

  it('keeps empty agent identifiers valid when switching back to None', () => {
    renderSubject('?blueprint=ai-agent-instance-blueprint');

    fireEvent.change(screen.getByLabelText('Instance Name'), { target: { value: 'Worker Two' } });
    selectAgentOption('Batch');
    selectAgentOption('None (compute only)');
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.queryByText(/Agent:/)).not.toBeInTheDocument();
  });

  it('keeps exposed ports configurable for Firecracker launches', () => {
    renderSubject('?blueprint=ai-agent-instance-blueprint');

    fireEvent.click(screen.getByRole('button', { name: 'Firecracker' }));
    const ports = screen.getByLabelText('Exposed Ports') as HTMLInputElement;

    expect(ports).not.toBeDisabled();
    expect(screen.getByText('Firecracker DNAT')).toBeInTheDocument();
  });

  it('exposes all-harness and computer-use capability controls', () => {
    renderSubject('?blueprint=ai-agent-instance-blueprint');

    expect(screen.getByRole('switch', { name: /All-Harness Runtime/i })).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: /Computer Use/i })).toBeInTheDocument();
  });

  it('turns a missing sandbox service into a direct service creation path', () => {
    infraStateRef.current = {
      blueprintId: '10',
      serviceId: '1',
      serviceValidated: false,
      serviceInfo: null,
    };
    serviceValidationRef.current = { serviceInfo: null, error: 'Service not found' };

    renderSubject('?blueprint=ai-agent-sandbox-blueprint');

    fireEvent.change(screen.getByLabelText('Sandbox Name'), { target: { value: 'Cloud Sandbox' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText('Service #1 not found')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Operators' }));
    expect(mockNavigate).toHaveBeenCalledWith('/operators');

    fireEvent.click(screen.getByRole('button', { name: /Create Service/i }));
    expect(screen.getByTestId('infra-modal')).toHaveTextContent('Infrastructure new');
  });
});
