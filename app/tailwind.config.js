/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: 'class',
  content: [
    "./src/index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      // Premium font stack optimized for crypto professionals
      fontFamily: {
        'sans': ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'Helvetica', 'Arial', 'sans-serif'],
        'display': ['Cabinet Grotesk', 'Inter', '-apple-system', 'system-ui', 'sans-serif'],
        'mono': ['JetBrains Mono', 'SF Mono', 'Consolas', 'Liberation Mono', 'Courier', 'monospace'],
        'serif': ['Newsreader', 'Georgia', 'Cambria', 'Times New Roman', 'Times', 'serif'],
      },

      // Elevated color system - Clean, light, professional
      colors: {
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

        // Primary - Complementary blue from Figma
        primary: {
          50: '#EFF6FF',
          100: '#DBEAFE',
          200: '#BFDBFE',
          300: '#93C5FD',
          400: '#60A5FA',
          500: '#2F6EF4',   // Complementary Blue (Figma)
          600: '#2563EB',   // Gradient end
          700: '#1D4ED8',   // Active state
          800: '#1E40AF',
          900: '#1E3A8A',
          950: '#172554',
        },

        // Sage - Success (from Figma: #34C759)
        sage: {
          50: '#F0FDF4',
          100: '#DCFCE7',
          200: '#BBF7D0',
          300: '#86EFAC',
          400: '#4ADE80',
          500: '#34C759',   // Success Green (Figma)
          600: '#16A34A',
          700: '#15803D',
          800: '#166534',
          900: '#14532D',
          950: '#052E16',
        },

        // Amber - Attention and caution (from Figma: #E8A728)
        amber: {
          50: '#FFFBEB',
          100: '#FEF3C7',
          200: '#FDE68A',
          300: '#FCD34D',
          400: '#FBBF24',
          500: '#E8A728',   // Alert Orange (Figma)
          600: '#D97706',
          700: '#B45309',
          800: '#92400E',
          900: '#78350F',
          950: '#451A03',
        },

        // Coral - Errors and dangers (from Figma: #EF4444)
        coral: {
          50: '#FEF2F2',
          100: '#FEE2E2',
          200: '#FECACA',
          300: '#FCA5A5',
          400: '#F87171',
          500: '#EF4444',   // Error Red (Figma)
          600: '#DC2626',
          700: '#B91C1C',
          800: '#991B1B',
          900: '#7F1D1D',
          950: '#450A0A',
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
