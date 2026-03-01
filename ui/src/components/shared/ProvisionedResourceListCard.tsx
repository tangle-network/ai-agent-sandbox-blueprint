import { Link } from 'react-router';
import { Card, CardContent } from '@tangle/blueprint-ui/components';
import { cn } from '@tangle/blueprint-ui';
import { ResourceIdentity } from './ResourceIdentity';

interface ProvisionedResourceListCardProps {
  to: string;
  name: string;
  status: string;
  teeEnabled?: boolean;
  image: string;
  specs: string;
  createdAt: number;
  iconClassName: string;
  iconContainerClassName: string;
  iconToneClassName: string;
  teeStyle?: 'pill' | 'text';
}

export function ProvisionedResourceListCard({
  to,
  name,
  status,
  teeEnabled,
  image,
  specs,
  createdAt,
  iconClassName,
  iconContainerClassName,
  iconToneClassName,
  teeStyle = 'pill',
}: ProvisionedResourceListCardProps) {
  return (
    <Link to={to}>
      <Card className="hover:border-cloud-elements-borderColor transition-all">
        <CardContent className="p-5">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-4 min-w-0">
              <div className={cn(
                'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
                iconContainerClassName,
              )}
              >
                <div className={cn(iconClassName, 'text-lg', iconToneClassName)} />
              </div>
              <ResourceIdentity
                name={name}
                status={status}
                teeEnabled={teeEnabled}
                image={image}
                specs={specs}
                teeStyle={teeStyle}
              />
            </div>
            <div className="flex items-center gap-3">
              <span className="text-xs font-data text-cloud-elements-textTertiary hidden md:block">
                {new Date(createdAt).toLocaleDateString()}
              </span>
              <div className="i-ph:caret-right text-lg text-cloud-elements-textTertiary" />
            </div>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}
