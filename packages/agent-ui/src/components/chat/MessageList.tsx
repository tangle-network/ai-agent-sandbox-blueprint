import { memo } from 'react';
import type { GroupedMessage } from '~/types/run';
import type { SessionPart } from '~/types/parts';
import type { Run } from '~/types/run';
import type { AgentBranding } from '~/types/branding';
import { RunGroup } from '../run/RunGroup';
import { UserMessage } from './UserMessage';

export interface MessageListProps {
  groups: GroupedMessage[];
  partMap: Record<string, SessionPart[]>;
  isCollapsed: (runId: string) => boolean;
  onToggleCollapse: (runId: string) => void;
  branding?: AgentBranding;
}

/**
 * Maps GroupedMessage[] to UserMessage and RunGroup components.
 * This is the main render list for the chat view.
 */
export const MessageList = memo(
  ({ groups, partMap, isCollapsed, onToggleCollapse, branding }: MessageListProps) => {
    return (
      <div className="space-y-3">
        {groups.map((group) => {
          if (group.type === 'user') {
            return (
              <UserMessage
                key={group.message.id}
                message={group.message}
                parts={partMap[group.message.id] ?? []}
              />
            );
          }

          return (
            <RunGroup
              key={group.run.id}
              run={group.run}
              partMap={partMap}
              collapsed={isCollapsed(group.run.id)}
              onToggle={() => onToggleCollapse(group.run.id)}
              branding={branding}
            />
          );
        })}
      </div>
    );
  },
);
MessageList.displayName = 'MessageList';
