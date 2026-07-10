/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: 'class',
  content: [
    "./src/index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      // Font roles — each maps to a CSS variable (defined in styles/tokens.css)
      // so themes can swap any role at runtime. The historical aliases
      // (sans/display) point at the matching role var for backwards compat.
      fontFamily: {
        'body': ['var(--font-body)'],
        'heading': ['var(--font-heading)'],
        'title': ['var(--font-title)'],
        'sans': ['var(--font-body)'],
        'display': ['var(--font-title)'],
        'mono': ['var(--font-mono)'],
        'serif': ['var(--font-serif)'],
      },

      // Elevated color system - Clean, light, professional
      colors: {
        // Command surface tokens — scoped to the ⌘K palette / help overlay.
        // Expand this set only with intent; the full reskin design system
        // is a separate decision.
        'cmd-surface':          'var(--cmd-surface)',
        'cmd-surface-elevated': 'var(--cmd-surface-elevated)',
        'cmd-foreground':       'var(--cmd-foreground)',
        'cmd-foreground-muted': 'var(--cmd-foreground-muted)',
        'cmd-border':           'var(--cmd-border)',
        'cmd-ring':             'var(--cmd-ring)',
        'cmd-accent':           'var(--cmd-accent)',
        'cmd-overlay':          'var(--cmd-overlay)',

        // ── Semantic theme tokens (var-backed, themeable at runtime) ─────────
        // Backed by channel vars in styles/tokens.css via rgb(... / <alpha-value>)
        // so opacity modifiers (bg-surface/50) keep working. These are the
        // canonical surface/text/border names the app migrates onto.
        surface: {
          DEFAULT:  'rgb(var(--surface) / <alpha-value>)',
          canvas:   'rgb(var(--surface-canvas) / <alpha-value>)',
          muted:    'rgb(var(--surface-muted) / <alpha-value>)',
          subtle:   'rgb(var(--surface-subtle) / <alpha-value>)',
          strong:   'rgb(var(--surface-strong) / <alpha-value>)',
          hover:    'rgb(var(--surface-hover) / <alpha-value>)',
          overlay:  'rgb(var(--surface-overlay) / <alpha-value>)',
        },
        content: {
          DEFAULT:   'rgb(var(--content) / <alpha-value>)',
          secondary: 'rgb(var(--content-secondary) / <alpha-value>)',
          muted:     'rgb(var(--content-muted) / <alpha-value>)',
          faint:     'rgb(var(--content-faint) / <alpha-value>)',
          inverted:  'rgb(var(--content-inverted) / <alpha-value>)',
        },
        line: {
          DEFAULT: 'rgb(var(--line) / <alpha-value>)',
          strong:  'rgb(var(--line-strong) / <alpha-value>)',
          subtle:  'rgb(var(--line-subtle) / <alpha-value>)',
        },

        // Neutral - Light theme grayscale (from Figma design tokens)
        neutral: {
          0: '#FFFFFF',     // Base / surface
          50: '#FAFAFA',
          100: '#F5F5F5',   // App background
          200: '#E5E5E5',
          300: '#D4D4D4',
          400: '#A3A3A3',
          500: '#737373',
          600: '#525252',
          700: '#404040',
          800: '#262626',
          900: '#171717',
          950: '#0A0A0A',
        },

        // Canvas - Background layers (mapped to neutral for compat)
        canvas: {
          50: '#FAFAFA',    // Base background
          100: '#F5F5F5',   // Secondary background
          150: '#EFEFEF',   // Tertiary background
          200: '#E5E5E5',   // Card background
          300: '#D4D4D4',   // Hover states
        },

        // Primary - Complementary blue from Figma. Var-backed (styles/tokens.css)
        // so the whole accent ramp is themeable without touching any classes.
        primary: {
          50:  'rgb(var(--primary-50) / <alpha-value>)',
          100: 'rgb(var(--primary-100) / <alpha-value>)',
          200: 'rgb(var(--primary-200) / <alpha-value>)',
          300: 'rgb(var(--primary-300) / <alpha-value>)',
          400: 'rgb(var(--primary-400) / <alpha-value>)',
          500: 'rgb(var(--primary-500) / <alpha-value>)',
          600: 'rgb(var(--primary-600) / <alpha-value>)',
          700: 'rgb(var(--primary-700) / <alpha-value>)',
          800: 'rgb(var(--primary-800) / <alpha-value>)',
          900: 'rgb(var(--primary-900) / <alpha-value>)',
          950: 'rgb(var(--primary-950) / <alpha-value>)',
        },

        // Sage - Success (from Figma: #34C759)
        sage: {
          50:  'rgb(var(--sage-50) / <alpha-value>)',
          100: 'rgb(var(--sage-100) / <alpha-value>)',
          200: 'rgb(var(--sage-200) / <alpha-value>)',
          300: 'rgb(var(--sage-300) / <alpha-value>)',
          400: 'rgb(var(--sage-400) / <alpha-value>)',
          500: 'rgb(var(--sage-500) / <alpha-value>)',
          600: 'rgb(var(--sage-600) / <alpha-value>)',
          700: 'rgb(var(--sage-700) / <alpha-value>)',
          800: 'rgb(var(--sage-800) / <alpha-value>)',
          900: 'rgb(var(--sage-900) / <alpha-value>)',
          950: 'rgb(var(--sage-950) / <alpha-value>)',
        },

        // Amber - Attention and caution (from Figma: #E8A728)
        amber: {
          50:  'rgb(var(--amber-50) / <alpha-value>)',
          100: 'rgb(var(--amber-100) / <alpha-value>)',
          200: 'rgb(var(--amber-200) / <alpha-value>)',
          300: 'rgb(var(--amber-300) / <alpha-value>)',
          400: 'rgb(var(--amber-400) / <alpha-value>)',
          500: 'rgb(var(--amber-500) / <alpha-value>)',
          600: 'rgb(var(--amber-600) / <alpha-value>)',
          700: 'rgb(var(--amber-700) / <alpha-value>)',
          800: 'rgb(var(--amber-800) / <alpha-value>)',
          900: 'rgb(var(--amber-900) / <alpha-value>)',
          950: 'rgb(var(--amber-950) / <alpha-value>)',
        },

        // Coral - Errors and dangers (from Figma: #EF4444)
        coral: {
          50:  'rgb(var(--coral-50) / <alpha-value>)',
          100: 'rgb(var(--coral-100) / <alpha-value>)',
          200: 'rgb(var(--coral-200) / <alpha-value>)',
          300: 'rgb(var(--coral-300) / <alpha-value>)',
          400: 'rgb(var(--coral-400) / <alpha-value>)',
          500: 'rgb(var(--coral-500) / <alpha-value>)',
          600: 'rgb(var(--coral-600) / <alpha-value>)',
          700: 'rgb(var(--coral-700) / <alpha-value>)',
          800: 'rgb(var(--coral-800) / <alpha-value>)',
          900: 'rgb(var(--coral-900) / <alpha-value>)',
          950: 'rgb(var(--coral-950) / <alpha-value>)',
        },

        // Stone - Neutral scale (keeping for backward compat, mapped to neutral)
        stone: {
          50: '#FAFAFA',
          100: '#F5F5F5',
          200: '#E5E5E5',
          300: '#D4D4D4',
          400: '#A3A3A3',
          500: '#737373',
          600: '#525252',
          700: '#404040',
          800: '#262626',
          900: '#171717',
          950: '#0A0A0A',
        },

        // Slate - Cool grays for data and charts
        slate: {
          50: '#F8FAFC',
          100: '#F1F5F9',
          200: '#E2E8F0',
          300: '#CBD5E1',
          400: '#94A3B8',
          500: '#64748B',
          600: '#475569',
          700: '#334155',
          800: '#1E293B',
          900: '#0F172A',
          950: '#020617',
        },

        // Market colors - For crypto specific UI
        market: {
          bullish: '#4DC46F',    // Green for gains
          bearish: '#F56565',    // Red for losses
          neutral: '#94A3B8',    // Gray for no change
          bitcoin: '#F7931A',    // Bitcoin orange
          ethereum: '#627EEA',   // Ethereum purple
          stablecoin: '#5B9BF3', // Blue for stables
        },

        // Accent colors for special elements
        accent: {
          lavender: '#9B8AFB',   // Premium features
          mint: '#6EE7B7',       // Achievements
          sky: '#7DD3FC',        // Notifications
          rose: '#FDA4AF',       // Alerts
          gold: '#FCD34D',       // Rewards
        }
      },

      // Refined spacing scale for elegant layouts
      spacing: {
        '4.5': '1.125rem',
        '13': '3.25rem',
        '15': '3.75rem',
        '17': '4.25rem',
        '18': '4.5rem',
        '22': '5.5rem',
        '30': '7.5rem',
        '34': '8.5rem',
        '42': '10.5rem',
        '68': '17rem',
        '76': '19rem',
        '84': '21rem',
        '88': '22rem',
        '92': '23rem',
        '128': '32rem',
        '144': '36rem',
      },

      // Sophisticated typography scale
      fontSize: {
        'micro': ['0.625rem', { lineHeight: '0.75rem', letterSpacing: '0.02em' }],
        'xs': ['0.75rem', { lineHeight: '1rem', letterSpacing: '0.01em' }],
        'sm': ['0.875rem', { lineHeight: '1.25rem', letterSpacing: '0' }],
        'base': ['1rem', { lineHeight: '1.5rem', letterSpacing: '-0.01em' }],
        'lg': ['1.125rem', { lineHeight: '1.75rem', letterSpacing: '-0.01em' }],
        'xl': ['1.25rem', { lineHeight: '1.875rem', letterSpacing: '-0.02em' }],
        '2xl': ['1.5rem', { lineHeight: '2rem', letterSpacing: '-0.02em' }],
        '3xl': ['1.875rem', { lineHeight: '2.25rem', letterSpacing: '-0.02em' }],
        '4xl': ['2.25rem', { lineHeight: '2.5rem', letterSpacing: '-0.03em' }],
        '5xl': ['3rem', { lineHeight: '3.5rem', letterSpacing: '-0.03em' }],
        '6xl': ['3.75rem', { lineHeight: '4rem', letterSpacing: '-0.04em' }],
        '7xl': ['4.5rem', { lineHeight: '4.75rem', letterSpacing: '-0.04em' }],
      },

      // Smooth border radius system
      borderRadius: {
        'xs': '0.25rem',
        'sm': '0.375rem',
        'md': '0.5rem',
        'lg': '0.625rem',
        'xl': '0.75rem',
        '2xl': '1rem',
        '3xl': '1.25rem',
        '4xl': '1.5rem',
        '5xl': '2rem',
      },

      // Sophisticated shadow system for depth
      boxShadow: {
        'glow': '0 0 20px rgba(91, 155, 243, 0.15)',
        'glow-lg': '0 0 40px rgba(91, 155, 243, 0.2)',
        'inner-glow': 'inset 0 0 20px rgba(91, 155, 243, 0.08)',
        'subtle': '0 1px 2px 0 rgba(0, 0, 0, 0.03), 0 1px 3px 0 rgba(0, 0, 0, 0.04)',
        'soft': '0 2px 8px -2px rgba(0, 0, 0, 0.08), 0 4px 12px -4px rgba(0, 0, 0, 0.08)',
        'medium': '0 4px 12px -2px rgba(0, 0, 0, 0.08), 0 8px 16px -4px rgba(0, 0, 0, 0.08)',
        'large': '0 8px 24px -4px rgba(0, 0, 0, 0.10), 0 16px 32px -8px rgba(0, 0, 0, 0.10)',
        'float': '0 12px 32px -8px rgba(0, 0, 0, 0.12), 0 24px 48px -12px rgba(0, 0, 0, 0.12)',
        'crisp': '0 0 0 1px rgba(0, 0, 0, 0.05), 0 2px 4px rgba(0, 0, 0, 0.08)',
        'cmd-palette': 'var(--cmd-shadow-palette)',
      },

      // Premium animations for polished interactions
      animation: {
        'fade-in': 'fadeIn 0.5s cubic-bezier(0.4, 0, 0.2, 1)',
        'fade-up': 'fadeUp 0.5s cubic-bezier(0.4, 0, 0.2, 1)',
        'slide-in': 'slideIn 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
        'slide-right': 'slideRight 0.3s cubic-bezier(0.25, 0.46, 0.45, 0.94)',
        'scale-in': 'scaleIn 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
        'shimmer': 'shimmer 2s linear infinite',
        'glow-pulse': 'glowPulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite',
        'float': 'float 3s ease-in-out infinite',
        'ticker': 'ticker 30s linear infinite',
        'wiggle': 'wiggle 0.5s ease-in-out',
        // Gentle ping-pong scroll for overflowing single-line labels (e.g. a
        // long thread-goal objective). Distance is supplied per-element via the
        // `--goal-marquee-shift` CSS var; `alternate` returns it to the start.
        'goal-marquee': 'goalMarquee 6s ease-in-out infinite alternate',
      },

      keyframes: {
        fadeIn: {
          '0%': { opacity: '0' },
          '100%': { opacity: '1' },
        },
        fadeUp: {
          '0%': { opacity: '0', transform: 'translateY(10px)' },
          '100%': { opacity: '1', transform: 'translateY(0)' },
        },
        slideIn: {
          '0%': { transform: 'translateX(-100%)' },
          '100%': { transform: 'translateX(0)' },
        },
        slideRight: {
          '0%': { opacity: '0', transform: 'translateX(100%)' },
          '100%': { opacity: '1', transform: 'translateX(0)' },
        },
        scaleIn: {
          '0%': { transform: 'scale(0.95)', opacity: '0' },
          '100%': { transform: 'scale(1)', opacity: '1' },
        },
        shimmer: {
          '0%': { backgroundPosition: '-200% 0' },
          '100%': { backgroundPosition: '200% 0' },
        },
        glowPulse: {
          '0%, 100%': { opacity: '1' },
          '50%': { opacity: '0.5' },
        },
        float: {
          '0%, 100%': { transform: 'translateY(0)' },
          '50%': { transform: 'translateY(-10px)' },
        },
        ticker: {
          '0%': { transform: 'translateX(0)' },
          '100%': { transform: 'translateX(-50%)' },
        },
        wiggle: {
          '0%, 100%': { transform: 'rotate(0deg)' },
          '25%': { transform: 'rotate(-9deg)' },
          '75%': { transform: 'rotate(9deg)' },
        },
        goalMarquee: {
          '0%, 18%': { transform: 'translateX(0)' },
          '82%, 100%': { transform: 'translateX(var(--goal-marquee-shift, 0px))' },
        },
      },

      // Backdrop blur for glass morphism
      backdropBlur: {
        'xs': '2px',
        'sm': '4px',
        'md': '8px',
        'lg': '12px',
        'xl': '16px',
        '2xl': '24px',
        '3xl': '40px',
      },

      // Background patterns and gradients
      backgroundImage: {
        'gradient-radial': 'radial-gradient(var(--tw-gradient-stops))',
        'gradient-conic': 'conic-gradient(from 180deg at 50% 50%, var(--tw-gradient-stops))',
        'gradient-mesh': 'linear-gradient(to right, #5B9BF3 0%, #9B8AFB 25%, #6EE7B7 50%, #7DD3FC 75%, #5B9BF3 100%)',
        'noise': "url('data:image/svg+xml,%3Csvg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"%3E%3Cfilter id=\"noise\"%3E%3CfeTurbulence type=\"fractalNoise\" baseFrequency=\"0.9\" numOctaves=\"4\" /%3E%3C/filter%3E%3Crect width=\"100\" height=\"100\" filter=\"url(%23noise)\" opacity=\"0.03\" /%3E%3C/svg%3E')",
      },

      // Extended transition duration for smooth animations
      transitionDuration: {
        '300': '300ms',
        '400': '400ms',
      },
    },
  },
  plugins: [
    require('@tailwindcss/forms'),
    require('@tailwindcss/typography'),
  ],
};
