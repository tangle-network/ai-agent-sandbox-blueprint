import { StorageWorkspace, type WorkspaceRailRow } from '~/components/console/ResourceWorkspacePanels';

interface StorageTabProps {
  rows: WorkspaceRailRow[];
  onSnapshot: () => void;
  snapshotEnabled: boolean;
}

export function StorageTab({ rows, onSnapshot, snapshotEnabled }: StorageTabProps) {
  return (
    <StorageWorkspace
      rows={rows}
      onSnapshot={onSnapshot}
      snapshotEnabled={snapshotEnabled}
    />
  );
}
