import { useCallback } from 'react';
import { useQuery } from '@tanstack/react-query';

import { useOperatorAuth } from '~/lib/hooks/useOperatorAuth';
import { getWorkflowOperatorUrl, type WorkflowScope } from '~/lib/workflows';

export interface WorkflowLatestExecution {
  executedAt: number;
  success: boolean;
  result: string;
  error: string;
  traceId: string;
  durationMs: number;
  inputTokens: number;
  outputTokens: number;
  sessionId: string;
}

export interface WorkflowRuntimeStatus {
  workflowId: number;
  targetStatus: 'available' | 'missing';
  runnable: boolean;
  running: boolean;
  lastRunAt: number | null;
  nextRunAt: number | null;
  latestExecution: WorkflowLatestExecution | null;
}

export interface WorkflowOperatorSummary extends WorkflowRuntimeStatus {
  scope: WorkflowScope;
  name: string;
  triggerType: string;
  triggerConfig: string;
  targetKind: number;
  targetSandboxId: string;
  targetServiceId: number;
  active: boolean;
}

export interface WorkflowOperatorDetail extends WorkflowOperatorSummary {
  workflowJson: string;
  sandboxConfigJson: string;
}

export function useWorkflowRuntimeStatus(
  scope: WorkflowScope,
  workflowId: string | null,
) {
  const operatorUrl = getWorkflowOperatorUrl(scope);
  const {
    getToken,
    getCachedToken,
    authCacheKey,
    isAuthenticated,
    isAuthenticating,
    error: authError,
  } = useOperatorAuth(operatorUrl);
  const cachedToken = getCachedToken();

  const fetchWithToken = useCallback(async <TData,>(path: string): Promise<TData> => {
    const request = async (token: string) =>
      fetch(`${operatorUrl}${path}`, {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      });

    let response = await request(cachedToken as string);

    if (response.status === 401) {
      const refreshedToken = await getToken(true);
      if (!refreshedToken) {
        throw new Error('Operator authentication required');
      }
      response = await request(refreshedToken);
    }

    if (response.status === 404) {
      return null as TData;
    }

    if (!response.ok) {
      const message = await response.text();
      throw new Error(message || `Workflow request failed (${response.status})`);
    }

    return response.json() as Promise<TData>;
  }, [cachedToken, getToken, operatorUrl]);

  const query = useQuery<WorkflowRuntimeStatus | null, Error>({
    queryKey: buildOwnerScopedWorkflowQueryKey(
      'workflow-runtime-status',
      operatorUrl,
      authCacheKey,
      workflowId,
    ),
    enabled: workflowId !== null && !!cachedToken,
    refetchInterval: 15_000,
    queryFn: async () => fetchWithToken<WorkflowRuntimeStatus | null>(`/api/workflows/${workflowId}`),
  });

  const authenticate = useCallback(async () => {
    const token = await getToken();
    return token;
  }, [getToken]);

  return {
    ...query,
    authenticate,
    authRequired: !cachedToken,
    isAuthenticated,
    isAuthenticating,
    authError,
    operatorUrl,
  };
}

export function useWorkflowSummaries(
  operatorUrl: string,
  enabled: boolean = true,
) {
  const {
    getToken,
    getCachedToken,
    authCacheKey,
    isAuthenticated,
    isAuthenticating,
    error: authError,
  } = useOperatorAuth(operatorUrl);
  const cachedToken = getCachedToken();

  const query = useQuery<WorkflowOperatorSummary[], Error>({
    queryKey: buildOwnerScopedWorkflowQueryKey(
      'workflow-summaries',
      operatorUrl,
      authCacheKey,
    ),
    enabled: enabled && !!cachedToken,
    refetchInterval: 15_000,
    queryFn: async () => {
      const response = await fetchWithWorkflowToken(operatorUrl, cachedToken, getToken, '/api/workflows');
      const body = await response.json() as { workflows?: WorkflowOperatorSummary[] };
      return body.workflows ?? [];
    },
  });

  const authenticate = useCallback(async () => {
    const token = await getToken();
    return token;
  }, [getToken]);

  return {
    ...query,
    authenticate,
    authRequired: !cachedToken,
    isAuthenticated,
    isAuthenticating,
    authError,
    operatorUrl,
  };
}

export function useWorkflowOperatorAccess(operatorUrl: string) {
  const {
    getToken,
    getCachedToken,
    authCacheKey,
    isAuthenticated,
    isAuthenticating,
    error: authError,
  } = useOperatorAuth(operatorUrl);

  const authenticate = useCallback(async () => {
    const token = await getToken();
    return token;
  }, [getToken]);

  return {
    operatorUrl,
    authCacheKey,
    getToken,
    getCachedToken,
    authenticate,
    authRequired: !getCachedToken(),
    isAuthenticated,
    isAuthenticating,
    authError,
  };
}

export function useWorkflowDetail(
  scope: WorkflowScope,
  workflowId: string | null,
) {
  const operatorUrl = getWorkflowOperatorUrl(scope);
  const {
    getToken,
    getCachedToken,
    authCacheKey,
    isAuthenticated,
    isAuthenticating,
    error: authError,
  } = useOperatorAuth(operatorUrl);
  const cachedToken = getCachedToken();

  const query = useQuery<WorkflowOperatorDetail | null, Error>({
    queryKey: buildOwnerScopedWorkflowQueryKey(
      'workflow-operator-detail',
      operatorUrl,
      authCacheKey,
      workflowId,
    ),
    enabled: workflowId !== null && !!cachedToken,
    refetchInterval: 15_000,
    queryFn: async () => {
      const response = await fetchWithWorkflowToken(
        operatorUrl,
        cachedToken,
        getToken,
        `/api/workflows/${workflowId}/detail`,
      );

      if (response.status === 404) {
        return null;
      }

      return response.json() as Promise<WorkflowOperatorDetail>;
    },
  });

  const authenticate = useCallback(async () => {
    const token = await getToken();
    return token;
  }, [getToken]);

  return {
    ...query,
    authenticate,
    authRequired: !cachedToken,
    isAuthenticated,
    isAuthenticating,
    authError,
    operatorUrl,
  };
}

function buildOwnerScopedWorkflowQueryKey(
  prefix: 'workflow-runtime-status' | 'workflow-summaries' | 'workflow-operator-detail',
  operatorUrl: string,
  authCacheKey: string | null,
  workflowId?: string | null,
) {
  if (workflowId === undefined) {
    return [prefix, operatorUrl, authCacheKey] as const;
  }

  return [prefix, operatorUrl, workflowId, authCacheKey] as const;
}

async function fetchWithWorkflowToken(
  operatorUrl: string,
  cachedToken: string | null,
  getToken: (forceRefresh?: boolean) => Promise<string | null>,
  path: string,
) {
  const request = async (token: string) =>
    fetch(`${operatorUrl}${path}`, {
      headers: {
        Authorization: `Bearer ${token}`,
      },
    });

  let response = await request(cachedToken as string);

  if (response.status === 401) {
    const refreshedToken = await getToken(true);
    if (!refreshedToken) {
      throw new Error('Operator authentication required');
    }
    response = await request(refreshedToken);
  }

  if (!response.ok && response.status !== 404) {
    const message = await response.text();
    throw new Error(message || `Workflow request failed (${response.status})`);
  }

  return response;
}
