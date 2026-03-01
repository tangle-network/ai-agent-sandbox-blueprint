import { useCallback } from 'react';

interface OperatorApiOpts {
  method?: string;
}

type GetTokenFn = (forceRefresh?: boolean) => Promise<string | null>;
type PathBuilderFn = (action: string) => string;

export function useOperatorApiCall(
  operatorUrl: string,
  getToken: GetTokenFn,
  buildPath: PathBuilderFn,
) {
  return useCallback(async (
    action: string,
    body?: Record<string, unknown>,
    opts?: OperatorApiOpts,
  ) => {
    const token = await getToken();
    if (!token) throw new Error('Wallet authentication required');

    const url = `${operatorUrl}${buildPath(action)}`;

    const doFetch = (bearerToken: string) =>
      fetch(url, {
        method: opts?.method ?? 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${bearerToken}`,
        },
        body: body ? JSON.stringify(body) : '{}',
      });

    let res = await doFetch(token);

    if (res.status === 401) {
      const freshToken = await getToken(true);
      if (!freshToken) throw new Error('Re-authentication failed');
      res = await doFetch(freshToken);
    }

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`${action} failed (${res.status}): ${text}`);
    }

    return res;
  }, [operatorUrl, getToken, buildPath]);
}
