import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StatusBadge } from './StatusBadge';

describe('StatusBadge', () => {
  it('renders the default label for a status', () => {
    render(<StatusBadge status="creating" />);
    expect(screen.getByText('Creating')).toBeInTheDocument();
  });

  it('allows the visible label to be overridden', () => {
    render(<StatusBadge status="creating" labelOverride="Pending activation" />);
    expect(screen.getByText('Pending activation')).toBeInTheDocument();
    expect(screen.queryByText('Creating')).not.toBeInTheDocument();
  });
});
