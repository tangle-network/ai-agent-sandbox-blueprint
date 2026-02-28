interface ConnectWalletCtaProps {
  onClick?: () => void;
  isReconnecting?: boolean;
}

export function ConnectWalletCta({ onClick, isReconnecting = false }: ConnectWalletCtaProps) {
  return (
    <button
      onClick={() => onClick?.()}
      disabled={!onClick}
      className="px-4 py-2.5 rounded-lg bg-violet-500/10 border border-violet-500/20 text-violet-700 dark:text-violet-400 text-sm font-display font-medium hover:bg-violet-500/20 transition-colors"
    >
      {isReconnecting ? (
        <span className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full border-2 border-violet-500/40 border-t-violet-600 dark:border-t-violet-400 animate-spin" />
          Reconnecting...
        </span>
      ) : 'Connect'}
    </button>
  );
}
