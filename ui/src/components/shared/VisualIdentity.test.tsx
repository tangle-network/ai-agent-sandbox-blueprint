import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { IdentityMark, OperatorIdentity, getRuntimeIdentity, getStackIdentity } from './VisualIdentity';

describe('VisualIdentity', () => {
  it('renders operator addresses as address-derived identicons', () => {
    const { container } = render(
      <OperatorIdentity address="0x1111111111111111111111111111111111111111" compact />,
    );

    const identicon = container.querySelector('img[src^="data:image/svg+xml;base64,"]');

    expect(identicon).toBeInTheDocument();
    expect(container.textContent).toContain('0x111111...111111');
  });

  it('does not stack secondary icon glyphs inside text marks', () => {
    const { container } = render(<IdentityMark identity={getRuntimeIdentity('docker')} />);

    const overlayIcon = Array.from(container.querySelectorAll('span')).find((element) =>
      String(element.getAttribute('class') ?? '').includes('i-ph:package'),
    );

    expect(overlayIcon).toBeUndefined();
  });

  it('uses icon marks instead of language initials for stack options', () => {
    const { container } = render(<IdentityMark identity={getStackIdentity('python')} />);
    const mark = container.querySelector('[title="Python"]');

    expect(mark?.textContent).toBe('');
    expect(mark?.querySelector('.i-ph\\:terminal-window')).toBeInTheDocument();
  });
});
