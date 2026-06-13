import { describe, expect, it } from 'vitest';

import { FONT_SIZE_PX, type FontSize } from '../store/themeSlice';
import { renderWithProviders } from '../test/test-utils';
import ThemeProvider from './ThemeProvider';

describe('<ThemeProvider />', () => {
  it.each<FontSize>(['small', 'medium', 'large', 'xlarge'])(
    'applies the %s font size to the root <html> element',
    fontSize => {
      renderWithProviders(
        <ThemeProvider>
          <span>child</span>
        </ThemeProvider>,
        { preloadedState: { theme: { mode: 'light', tabBarLabels: 'hover', fontSize } } }
      );

      expect(document.documentElement.style.fontSize).toBe(FONT_SIZE_PX[fontSize]);
    }
  );

  it('renders its children', () => {
    const { getByText } = renderWithProviders(
      <ThemeProvider>
        <span>hello</span>
      </ThemeProvider>,
      { preloadedState: { theme: { mode: 'light', tabBarLabels: 'hover', fontSize: 'medium' } } }
    );

    expect(getByText('hello')).toBeInTheDocument();
  });
});
