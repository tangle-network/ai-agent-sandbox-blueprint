/**
 * useCreateDeploy — unified deploy hook for the create wizard.
 *
 * Abstracts the two deployment paths behind a single state machine:
 *   Path A (sandbox / instance with existing service): submitJob → watch provision
 *   Path B (instance without service): requestService → watch ServiceActivated → watch OperatorProvisioned
 *
 * The hook owns all TX lifecycle state, store updates, and provision watching,
 * so the consuming component only needs to render based on `status` + `provision`.
 */

import { useState, useCallback, useEffect, useMemo } from 'react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt } from 'wagmi';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { tangleServicesAbi } from '~/lib/contracts/abi';
import { getAddresses } from '~/lib/contracts/publicClient';
import { useSubmitJob, type JobSubmitStatus } from '~/lib/hooks/useSubmitJob';
import { useOperators, type DiscoveredOperator } from '~/lib/hooks/useOperators';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { addSandbox, updateSandboxStatus } from '~/lib/stores/sandboxes';
import { addInstance, updateInstanceStatus } from '~/lib/stores/instances';
import type { BlueprintDefinition, JobDefinition } from '~/lib/blueprints';
import type { SandboxAddresses } from '~/lib/contracts/chains';
import type { InfraConfig } from '~/lib/stores/infra';
import type { Address } from 'viem';

// ── Public types ──

export type DeployMode = 'sandbox' | 'instance';

export type DeployStatus =
  | 'idle'           // Ready to deploy
  | 'signing'        // Waiting for wallet confirmation
  | 'pending'        // TX submitted, awaiting on-chain confirmation
  | 'confirmed'      // TX confirmed; for sandbox → provisioning; for instance → waiting for operator
  | 'provisioning'   // Sandbox: operator is provisioning; Instance: operator provisioning in progress
  | 'ready'          // Fully provisioned (sandbox or instance)
  | 'failed';        // TX failed

export interface ProvisionInfo {
  sandboxId: string;
  sidecarUrl: string;
}

export interface CreateDeployState {
  status: DeployStatus;
  txHash?: `0x${string}`;
  error?: string;
  callId?: number;
  provision?: ProvisionInfo;
  /** Discovered operators (only relevant for instance mode without existing service) */
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  /** Whether we're creating a new service (instance Path B) vs submitting a job (Path A) */
  isNewService: boolean;
}

interface UseCreateDeployOpts {
  blueprint: BlueprintDefinition | undefined;
  job: JobDefinition | null;
  values: Record<string, unknown>;
  infra: InfraConfig;
  validate: () => boolean;
}

// ── Hook ──

export function useCreateDeploy({ blueprint, job, values, infra, validate }: UseCreateDeployOpts) {
  const { address } = useAccount();

  // Derive mode from blueprint ID
  const mode: DeployMode = blueprint?.id === 'ai-agent-sandbox-blueprint' ? 'sandbox' : 'instance';
  const isTeeInstance = blueprint?.id === 'ai-agent-tee-instance-blueprint';
  const isInstanceMode = mode === 'instance';

  // Path A: submitJob (sandbox, or instance with existing service)
  const {
    submitJob, status: jobStatus, error: jobError, txHash: jobTxHash, callId, reset: resetJob,
  } = useSubmitJob();

  // Path B: requestService (instance without existing service)
  const {
    writeContractAsync: requestServiceWrite,
    data: serviceTxHash,
    isPending: serviceSigning,
    reset: resetServiceTx,
  } = useWriteContract();
  const {
    isSuccess: serviceConfirmed,
    isLoading: serviceTxPending,
  } = useWaitForTransactionReceipt({ hash: serviceTxHash });

  // Service error tracking for Path B
  const [serviceError, setServiceError] = useState<string | null>(null);

  // Operator discovery (instance mode)
  const { operators, isLoading: operatorsLoading } = useOperators(
    isInstanceMode ? BigInt(infra.blueprintId || '0') : 0n,
  );

  // Check if existing service is valid (cached in infra store)
  const hasValidService = !!(
    infra.serviceInfo?.active &&
    infra.serviceInfo?.permitted &&
    infra.serviceId
  );

  // Whether we're creating a new service (Path B) vs submitting a job (Path A)
  const isNewService = isInstanceMode && !hasValidService;

  // ── Unified status ──

  const status = useMemo<DeployStatus>(() => {
    // Path A status
    if (!isNewService) {
      if (jobStatus === 'signing') return 'signing';
      if (jobStatus === 'pending') return 'pending';
      if (jobStatus === 'failed') return 'failed';
      if (jobStatus === 'confirmed') return 'confirmed';
      return 'idle';
    }

    // Path B status
    if (serviceError) return 'failed';
    if (serviceSigning) return 'signing';
    if (serviceTxPending) return 'pending';
    if (serviceConfirmed) return 'confirmed';
    return 'idle';
  }, [isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError]);

  const txHash = isNewService ? serviceTxHash : jobTxHash;
  const error = isNewService ? serviceError : jobError;

  // ── Instance provision watching ──

  const instanceProvision = useInstanceProvisionWatcher(
    infra.serviceId ? BigInt(infra.serviceId) : null,
    isTeeInstance ? 'tee-instance' : 'instance',
    status === 'confirmed' && isInstanceMode,
  );

  // ── Store updates ──

  // Update store when submitJob callId is parsed from receipt
  useEffect(() => {
    if (callId != null) {
      const name = String(values.name || '');
      if (!name) return;
      if (mode === 'sandbox') {
        updateSandboxStatus(name, 'creating', { callId });
      } else {
        updateInstanceStatus(name, 'creating', { callId });
      }
    }
  }, [callId]);

  // Add instance to store when requestService confirms
  useEffect(() => {
    if (serviceConfirmed && isInstanceMode) {
      const name = String(values.name || '');
      if (!name) return;
      addInstance({
        id: name,
        name,
        image: String(values.image || ''),
        cpuCores: Number(values.cpuCores) || 2,
        memoryMb: Number(values.memoryMb) || 2048,
        diskGb: Number(values.diskGb) || 10,
        createdAt: Date.now(),
        blueprintId: infra.blueprintId,
        serviceId: infra.serviceId || '',
        teeEnabled: isTeeInstance,
        status: 'creating',
        txHash: serviceTxHash,
      });
    }
  }, [serviceConfirmed]);

  // Update store when instance provision event arrives
  useEffect(() => {
    if (instanceProvision) {
      const name = String(values.name || '');
      if (name) {
        updateInstanceStatus(name, 'running', {
          id: instanceProvision.sandboxId,
          sidecarUrl: instanceProvision.sidecarUrl,
        });
      }
    }
  }, [instanceProvision]);

  // ── Deploy action ──

  const deploy = useCallback(async () => {
    if (!job || !validate()) return;

    const args = encodeJobArgs(job, values);
    const name = String(values.name || '');

    // Path A: Submit job to existing service
    if (!isNewService) {
      const hash = await submitJob({
        serviceId: BigInt(infra.serviceId || '0'),
        jobId: job.id,
        args,
        label: `${job.label}: ${name}`,
        value: BigInt(job.pricingMultiplier ?? 50) * 1_000_000_000_000_000n,
      });

      if (hash) {
        const common = {
          id: name,
          name,
          image: String(values.image || ''),
          cpuCores: Number(values.cpuCores) || 2,
          memoryMb: Number(values.memoryMb) || 2048,
          diskGb: Number(values.diskGb) || 10,
          createdAt: Date.now(),
          blueprintId: infra.blueprintId,
          serviceId: infra.serviceId,
          status: 'creating' as const,
          txHash: hash,
        };
        if (mode === 'sandbox') {
          addSandbox(common);
        } else {
          addInstance({ ...common, teeEnabled: isTeeInstance });
        }
      }
      return;
    }

    // Path B: Create new service with config as requestInputs
    if (!address) return;
    const addrs = getAddresses<SandboxAddresses>();
    const ops = operators.map((o) => o.address);
    if (ops.length === 0) return;

    try {
      setServiceError(null);
      await requestServiceWrite({
        address: addrs.services,
        abi: tangleServicesAbi,
        functionName: 'requestService',
        args: [
          BigInt(infra.blueprintId),
          ops,
          args as `0x${string}`,
          [address as Address],
          864000n, // ~30 days at 3s blocks
          '0x0000000000000000000000000000000000000000' as Address,
          0n,
        ],
      });
    } catch (err: any) {
      setServiceError(err?.shortMessage ?? err?.message ?? 'Service creation failed');
    }
  }, [job, values, infra, validate, isNewService, submitJob, address, operators, requestServiceWrite, mode, isTeeInstance]);

  // ── Reset ──

  const reset = useCallback(() => {
    resetJob();
    resetServiceTx();
    setServiceError(null);
  }, [resetJob, resetServiceTx]);

  // ── Computed flags ──

  const canDeploy = !!(
    job &&
    values.name &&
    address &&
    status === 'idle' &&
    (mode === 'sandbox' ? hasValidService : true) &&
    (!isNewService || (operators.length > 0 && !operatorsLoading))
  );

  return {
    // State
    mode,
    status,
    txHash,
    error,
    callId: callId ?? undefined,
    provision: instanceProvision ?? undefined,
    operators,
    operatorsLoading,
    isNewService,
    isInstanceMode,
    isTeeInstance,
    hasValidService,
    canDeploy,
    // Actions
    deploy,
    reset,
  };
}
