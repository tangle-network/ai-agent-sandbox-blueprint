/** Shared environment-driven configuration constants. */

export const OPERATOR_API_URL =
  import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';

export const INSTANCE_OPERATOR_API_URL =
  import.meta.env.VITE_INSTANCE_OPERATOR_API_URL ?? '';
