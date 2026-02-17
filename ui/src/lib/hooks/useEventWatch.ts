import { useEffect } from 'react';
import { useWatchContractEvent } from 'wagmi';
import { agentSandboxBlueprintAbi, tangleJobsAbi } from '~/lib/contracts/abi';
import { getAddresses } from '~/lib/contracts/publicClient';

/**
 * Watch for SandboxCreated events from the blueprint contract.
 */
export function useSandboxCreatedEvents(
  onEvent: (sandboxHash: `0x${string}`, operator: `0x${string}`) => void,
) {
  const addrs = getAddresses();
  useWatchContractEvent({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    eventName: 'SandboxCreated',
    onLogs(logs) {
      for (const log of logs) {
        const { sandboxHash, operator } = log.args as {
          sandboxHash: `0x${string}`;
          operator: `0x${string}`;
        };
        if (sandboxHash && operator) {
          onEvent(sandboxHash, operator);
        }
      }
    },
  });
}

/**
 * Watch for SandboxDeleted events from the blueprint contract.
 */
export function useSandboxDeletedEvents(
  onEvent: (sandboxHash: `0x${string}`, operator: `0x${string}`) => void,
) {
  const addrs = getAddresses();
  useWatchContractEvent({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    eventName: 'SandboxDeleted',
    onLogs(logs) {
      for (const log of logs) {
        const { sandboxHash, operator } = log.args as {
          sandboxHash: `0x${string}`;
          operator: `0x${string}`;
        };
        if (sandboxHash && operator) {
          onEvent(sandboxHash, operator);
        }
      }
    },
  });
}

/**
 * Watch for WorkflowStored events.
 */
export function useWorkflowStoredEvents(
  onEvent: (workflowId: bigint, triggerType: string, triggerConfig: string) => void,
) {
  const addrs = getAddresses();
  useWatchContractEvent({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    eventName: 'WorkflowStored',
    onLogs(logs) {
      for (const log of logs) {
        const args = log.args as {
          workflow_id: bigint;
          trigger_type: string;
          trigger_config: string;
        };
        if (args.workflow_id !== undefined) {
          onEvent(args.workflow_id, args.trigger_type, args.trigger_config);
        }
      }
    },
  });
}

/**
 * Watch for JobResultReceived events on the Jobs contract.
 */
export function useJobResultEvents(
  onEvent: (serviceId: bigint, job: number, callId: bigint, operator: `0x${string}`, outputs: `0x${string}`) => void,
) {
  const addrs = getAddresses();
  useWatchContractEvent({
    address: addrs.jobs,
    abi: tangleJobsAbi,
    eventName: 'JobResultReceived',
    onLogs(logs) {
      for (const log of logs) {
        const args = log.args as {
          serviceId: bigint;
          job: number;
          callId: bigint;
          operator: `0x${string}`;
          outputs: `0x${string}`;
        };
        if (args.serviceId !== undefined) {
          onEvent(args.serviceId, args.job, args.callId, args.operator, args.outputs);
        }
      }
    },
  });
}
