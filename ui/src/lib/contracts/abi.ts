/**
 * AgentSandboxBlueprint ABI — extracted from AgentSandboxBlueprint.sol
 * Includes all view functions, events, errors, and job metadata.
 */
export const agentSandboxBlueprintAbi = [
  // ── Constructor ──
  {
    type: 'constructor',
    inputs: [{ name: 'restakingAddress', type: 'address' }],
    stateMutability: 'nonpayable',
  },

  // ── Events ──
  {
    type: 'event',
    name: 'OperatorAssigned',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
    ],
  },
  {
    type: 'event',
    name: 'OperatorRouted',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
    ],
  },
  {
    type: 'event',
    name: 'SandboxCreated',
    inputs: [
      { name: 'sandboxHash', type: 'bytes32', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
    ],
  },
  {
    type: 'event',
    name: 'SandboxDeleted',
    inputs: [
      { name: 'sandboxHash', type: 'bytes32', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
    ],
  },
  {
    type: 'event',
    name: 'WorkflowStored',
    inputs: [
      { name: 'workflow_id', type: 'uint64', indexed: true },
      { name: 'trigger_type', type: 'string', indexed: false },
      { name: 'trigger_config', type: 'string', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'WorkflowTriggered',
    inputs: [
      { name: 'workflow_id', type: 'uint64', indexed: true },
      { name: 'triggered_at', type: 'uint64', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'WorkflowCanceled',
    inputs: [
      { name: 'workflow_id', type: 'uint64', indexed: true },
      { name: 'canceled_at', type: 'uint64', indexed: false },
    ],
  },

  // ── Errors ──
  { type: 'error', name: 'NoAvailableCapacity', inputs: [] },
  {
    type: 'error',
    name: 'OperatorMismatch',
    inputs: [
      { name: 'expected', type: 'address' },
      { name: 'actual', type: 'address' },
    ],
  },
  {
    type: 'error',
    name: 'SandboxNotFound',
    inputs: [{ name: 'sandboxHash', type: 'bytes32' }],
  },
  {
    type: 'error',
    name: 'SandboxAlreadyExists',
    inputs: [{ name: 'sandboxHash', type: 'bytes32' }],
  },

  // ── Job metadata (pure) ──
  {
    type: 'function',
    name: 'jobIds',
    inputs: [],
    outputs: [{ name: 'ids', type: 'uint8[]' }],
    stateMutability: 'pure',
  },
  {
    type: 'function',
    name: 'supportsJob',
    inputs: [{ name: 'jobId', type: 'uint8' }],
    outputs: [{ type: 'bool' }],
    stateMutability: 'pure',
  },
  {
    type: 'function',
    name: 'jobCount',
    inputs: [],
    outputs: [{ type: 'uint256' }],
    stateMutability: 'pure',
  },

  // ── Capacity / operator views ──
  {
    type: 'function',
    name: 'defaultMaxCapacity',
    inputs: [],
    outputs: [{ type: 'uint32' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'totalActiveSandboxes',
    inputs: [],
    outputs: [{ type: 'uint32' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'operatorMaxCapacity',
    inputs: [{ name: 'operator', type: 'address' }],
    outputs: [{ type: 'uint32' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'operatorActiveSandboxes',
    inputs: [{ name: 'operator', type: 'address' }],
    outputs: [{ type: 'uint32' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getOperatorLoad',
    inputs: [{ name: 'operator', type: 'address' }],
    outputs: [
      { name: 'active', type: 'uint32' },
      { name: 'max', type: 'uint32' },
    ],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getAvailableCapacity',
    inputs: [],
    outputs: [{ name: 'available', type: 'uint32' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getServiceStats',
    inputs: [],
    outputs: [
      { name: 'totalSandboxes', type: 'uint32' },
      { name: 'totalCapacity', type: 'uint32' },
    ],
    stateMutability: 'view',
  },

  // ── Sandbox registry views ──
  {
    type: 'function',
    name: 'sandboxOperator',
    inputs: [{ name: 'sandboxHash', type: 'bytes32' }],
    outputs: [{ type: 'address' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'sandboxActive',
    inputs: [{ name: 'sandboxHash', type: 'bytes32' }],
    outputs: [{ type: 'bool' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getSandboxOperator',
    inputs: [{ name: 'sandboxId', type: 'string' }],
    outputs: [{ type: 'address' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'isSandboxActive',
    inputs: [{ name: 'sandboxId', type: 'string' }],
    outputs: [{ type: 'bool' }],
    stateMutability: 'view',
  },

  // ── Workflow views ──
  {
    type: 'function',
    name: 'getWorkflow',
    inputs: [{ name: 'workflowId', type: 'uint64' }],
    outputs: [
      {
        name: '',
        type: 'tuple',
        components: [
          { name: 'name', type: 'string' },
          { name: 'workflow_json', type: 'string' },
          { name: 'trigger_type', type: 'string' },
          { name: 'trigger_config', type: 'string' },
          { name: 'sandbox_config_json', type: 'string' },
          { name: 'active', type: 'bool' },
          { name: 'created_at', type: 'uint64' },
          { name: 'updated_at', type: 'uint64' },
          { name: 'last_triggered_at', type: 'uint64' },
        ],
      },
    ],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getWorkflowIds',
    inputs: [{ name: 'activeOnly', type: 'bool' }],
    outputs: [{ name: 'ids', type: 'uint64[]' }],
    stateMutability: 'view',
  },

  // ── Pricing helpers (pure) ──
  {
    type: 'function',
    name: 'getDefaultJobRates',
    inputs: [{ name: 'baseRate', type: 'uint256' }],
    outputs: [
      { name: 'jobIndexes', type: 'uint8[]' },
      { name: 'rates', type: 'uint256[]' },
    ],
    stateMutability: 'pure',
  },
  {
    type: 'function',
    name: 'getJobPriceMultiplier',
    inputs: [{ name: 'jobId', type: 'uint8' }],
    outputs: [{ type: 'uint256' }],
    stateMutability: 'pure',
  },

  // ── Admin functions ──
  {
    type: 'function',
    name: 'setDefaultMaxCapacity',
    inputs: [{ name: 'capacity', type: 'uint32' }],
    outputs: [],
    stateMutability: 'nonpayable',
  },
  {
    type: 'function',
    name: 'setOperatorCapacity',
    inputs: [
      { name: 'operator', type: 'address' },
      { name: 'capacity', type: 'uint32' },
    ],
    outputs: [],
    stateMutability: 'nonpayable',
  },

  // ── Tangle hooks (called by the system, included for event decoding) ──
  {
    type: 'function',
    name: 'onJobCall',
    inputs: [
      { name: 'serviceId', type: 'uint64' },
      { name: 'job', type: 'uint8' },
      { name: 'jobCallId', type: 'uint64' },
      { name: 'inputs', type: 'bytes' },
    ],
    outputs: [],
    stateMutability: 'payable',
  },
  {
    type: 'function',
    name: 'onJobResult',
    inputs: [
      { name: 'serviceId', type: 'uint64' },
      { name: 'job', type: 'uint8' },
      { name: 'jobCallId', type: 'uint64' },
      { name: 'operator', type: 'address' },
      { name: 'inputs', type: 'bytes' },
      { name: 'outputs', type: 'bytes' },
    ],
    outputs: [],
    stateMutability: 'payable',
  },
] as const;

/**
 * Tangle Jobs precompile ABI — minimal interface for submitting jobs.
 */
export const tangleJobsAbi = [
  {
    type: 'function',
    name: 'submitJob',
    inputs: [
      { name: 'serviceId', type: 'uint64' },
      { name: 'job', type: 'uint8' },
      { name: 'args', type: 'bytes' },
    ],
    outputs: [{ name: 'callId', type: 'uint64' }],
    stateMutability: 'payable',
  },
  {
    type: 'event',
    name: 'JobCalled',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'job', type: 'uint8', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
      { name: 'caller', type: 'address', indexed: false },
      { name: 'args', type: 'bytes', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'JobResultReceived',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'job', type: 'uint8', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
      { name: 'operator', type: 'address', indexed: false },
      { name: 'outputs', type: 'bytes', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'JobSubmitted',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
      { name: 'jobIndex', type: 'uint8', indexed: true },
      { name: 'caller', type: 'address', indexed: false },
      { name: 'inputs', type: 'bytes', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'JobCompleted',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
    ],
  },
  {
    type: 'event',
    name: 'JobResultSubmitted',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'callId', type: 'uint64', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
      { name: 'output', type: 'bytes', indexed: false },
    ],
  },
] as const;

/**
 * Tangle Services precompile ABI — service creation, management, and validation.
 */
export const tangleServicesAbi = [
  {
    type: 'function',
    name: 'requestService',
    inputs: [
      { name: 'blueprintId', type: 'uint64' },
      { name: 'operators', type: 'address[]' },
      { name: 'config', type: 'bytes' },
      { name: 'permittedCallers', type: 'address[]' },
      { name: 'ttl', type: 'uint64' },
      { name: 'paymentToken', type: 'address' },
      { name: 'paymentAmount', type: 'uint256' },
    ],
    outputs: [{ name: 'requestId', type: 'uint64' }],
    stateMutability: 'payable',
  },
  {
    type: 'function',
    name: 'createServiceFromQuotes',
    inputs: [
      { name: 'blueprintId', type: 'uint64' },
      {
        name: 'quotes',
        type: 'tuple[]',
        components: [
          {
            name: 'details',
            type: 'tuple',
            components: [
              { name: 'blueprintId', type: 'uint64' },
              { name: 'ttlBlocks', type: 'uint64' },
              { name: 'totalCost', type: 'uint256' },
              { name: 'timestamp', type: 'uint64' },
              { name: 'expiry', type: 'uint64' },
              {
                name: 'securityCommitments',
                type: 'tuple[]',
                components: [
                  {
                    name: 'asset',
                    type: 'tuple',
                    components: [
                      { name: 'kind', type: 'uint8' },
                      { name: 'token', type: 'address' },
                    ],
                  },
                  { name: 'exposureBps', type: 'uint16' },
                ],
              },
            ],
          },
          { name: 'signature', type: 'bytes' },
          { name: 'operator', type: 'address' },
        ],
      },
      { name: 'config', type: 'bytes' },
      { name: 'permittedCallers', type: 'address[]' },
      { name: 'ttl', type: 'uint64' },
    ],
    outputs: [{ name: 'serviceId', type: 'uint64' }],
    stateMutability: 'payable',
  },
  {
    type: 'function',
    name: 'getService',
    inputs: [{ name: 'serviceId', type: 'uint64' }],
    outputs: [
      {
        name: '',
        type: 'tuple',
        components: [
          { name: 'blueprintId', type: 'uint64' },
          { name: 'owner', type: 'address' },
          { name: 'createdAt', type: 'uint64' },
          { name: 'ttl', type: 'uint64' },
          { name: 'terminatedAt', type: 'uint64' },
          { name: 'lastPaymentAt', type: 'uint64' },
          { name: 'operatorCount', type: 'uint32' },
          { name: 'minOperators', type: 'uint32' },
          { name: 'maxOperators', type: 'uint32' },
          { name: 'membership', type: 'uint8' },
          { name: 'pricing', type: 'uint8' },
          { name: 'status', type: 'uint8' },
        ],
      },
    ],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'isServiceActive',
    inputs: [{ name: 'serviceId', type: 'uint64' }],
    outputs: [{ name: '', type: 'bool' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getServiceOperators',
    inputs: [{ name: 'serviceId', type: 'uint64' }],
    outputs: [{ name: '', type: 'address[]' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'isPermittedCaller',
    inputs: [
      { name: 'serviceId', type: 'uint64' },
      { name: 'caller', type: 'address' },
    ],
    outputs: [{ name: '', type: 'bool' }],
    stateMutability: 'view',
  },
  {
    type: 'event',
    name: 'ServiceRequested',
    inputs: [
      { name: 'requester', type: 'address', indexed: true },
      { name: 'requestId', type: 'uint64', indexed: true },
      { name: 'blueprintId', type: 'uint64', indexed: true },
    ],
  },
  {
    type: 'event',
    name: 'ServiceActivated',
    inputs: [
      { name: 'serviceId', type: 'uint64', indexed: true },
      { name: 'requestId', type: 'uint64', indexed: true },
      { name: 'blueprintId', type: 'uint64', indexed: true },
    ],
  },
] as const;

/**
 * Tangle Operators precompile ABI — operator discovery and preference reads.
 */
export const tangleOperatorsAbi = [
  {
    type: 'function',
    name: 'blueprintOperatorCount',
    inputs: [{ name: 'blueprintId', type: 'uint64' }],
    outputs: [{ name: '', type: 'uint256' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'isOperatorRegistered',
    inputs: [
      { name: 'blueprintId', type: 'uint64' },
      { name: 'operator', type: 'address' },
    ],
    outputs: [{ name: '', type: 'bool' }],
    stateMutability: 'view',
  },
  {
    type: 'function',
    name: 'getOperatorPreferences',
    inputs: [
      { name: 'blueprintId', type: 'uint64' },
      { name: 'operator', type: 'address' },
    ],
    outputs: [
      {
        name: 'preferences',
        type: 'tuple',
        components: [
          { name: 'ecdsaPublicKey', type: 'bytes' },
          { name: 'rpcAddress', type: 'string' },
        ],
      },
    ],
    stateMutability: 'view',
  },
  {
    type: 'event',
    name: 'OperatorRegistered',
    inputs: [
      { name: 'blueprintId', type: 'uint64', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
      { name: 'ecdsaPublicKey', type: 'bytes', indexed: false },
      { name: 'rpcAddress', type: 'string', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'OperatorUnregistered',
    inputs: [
      { name: 'blueprintId', type: 'uint64', indexed: true },
      { name: 'operator', type: 'address', indexed: true },
    ],
  },
] as const;
