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

  it('applies an active custom theme — colour vars, font vars, and .dark', () => {
    renderWithProviders(
      <ThemeProvider>
        <span>themed</span>
      </ThemeProvider>,
      {
        preloadedState: {
          theme: {
            mode: 'dark',
            tabBarLabels: 'hover',
            fontSize: 'medium',
            activeThemeId: 'custom-1',
            customThemes: [
              {
                id: 'custom-1',
                name: 'C',
                isDark: true,
                builtIn: false,
                colors: { surface: '1 2 3' },
                fonts: { body: 'TestFont, sans-serif' },
              },
            ],
          },
        },
      }
    );

    const root = document.documentElement;
    expect(root.classList.contains('dark')).toBe(true);
    expect(root.style.getPropertyValue('--surface')).toBe('1 2 3');
    expect(root.style.getPropertyValue('--font-body')).toBe('TestFont, sans-serif');
  });
});
