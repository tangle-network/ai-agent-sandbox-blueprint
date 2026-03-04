import type { ReactNode } from 'react';
import { Link } from 'react-router';
import {
  AnimatedPage,
  Card,
  CardContent,
  StaggerContainer,
  StaggerItem,
  Button,
} from '@tangle-network/blueprint-ui/components';

interface ProvisionedResourceIndexPageProps<T> {
  title: string;
  subtitle: string;
  createTo: string;
  createLabel: string;
  items: T[];
  getKey: (item: T) => string;
  renderItem: (item: T) => ReactNode;
  emptyIconClassName: string;
  emptyTitle: string;
  emptySubtitle: string;
  emptyCreateTo: string;
  emptyCreateLabel: string;
}

export function ProvisionedResourceIndexPage<T>({
  title,
  subtitle,
  createTo,
  createLabel,
  items,
  getKey,
  renderItem,
  emptyIconClassName,
  emptyTitle,
  emptySubtitle,
  emptyCreateTo,
  emptyCreateLabel,
}: ProvisionedResourceIndexPageProps<T>) {
  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">{title}</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">{subtitle}</p>
        </div>
        <Link to={createTo}>
          <Button>
            <div className="i-ph:plus text-base" />
            {createLabel}
          </Button>
        </Link>
      </div>

      {items.length > 0 ? (
        <StaggerContainer className="space-y-3">
          {items.map((item) => (
            <StaggerItem key={getKey(item)}>{renderItem(item)}</StaggerItem>
          ))}
        </StaggerContainer>
      ) : (
        <Card>
          <CardContent className="p-6">
            <div className="py-16 text-center">
              <div className={`${emptyIconClassName} text-4xl text-cloud-elements-textTertiary mb-3 mx-auto`} />
              <p className="text-cloud-elements-textSecondary font-display">{emptyTitle}</p>
              <p className="text-sm text-cloud-elements-textTertiary mt-1">{emptySubtitle}</p>
              <Link to={emptyCreateTo} className="inline-block mt-4">
                <Button variant="outline" size="sm">{emptyCreateLabel}</Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}
