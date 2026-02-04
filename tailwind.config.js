/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: 'class',
  content: [
    "./index.html",
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

      // Elevated color system - Calm, trustworthy, and sophisticated
      colors: {
        // Canvas - Background layers with subtle warmth
        canvas: {
          50: '#FAFAF9',    // Base background
          100: '#F5F5F4',   // Secondary background
          150: '#EDEDEC',   // Tertiary background
          200: '#E5E5E3',   // Card background
          300: '#D4D4D1',   // Hover states
        },

        // Primary - Premium ocean blue with depth (optimized for dark backgrounds)
        primary: {
          50: '#F0F7FF',
          100: '#E0EFFF',
          200: '#C7E2FF',
          300: '#A5D0FF',
          400: '#7AB5FF',
          500: '#4A83DD',   // Main brand - darker blue for dark backgrounds
          600: '#3D6DC4',   // Hover state
          700: '#345A9F',   // Active state
          800: '#2D4B7F',
          900: '#1E3052',
          950: '#0F1A2E',
        },

        // Sage - Success and growth (sophisticated green)
        sage: {
          50: '#F7FDF9',
          100: '#ECFAEF',
          200: '#D4F4DC',
          300: '#AEEAC1',
          400: '#72D892',
          500: '#4DC46F',   // Main success - refined green
          600: '#3BA858',
          700: '#318B48',
          800: '#2B6F3C',
          900: '#255933',
          950: '#14371E',
        },

        // Amber - Attention and caution (warm, muted)
        amber: {
          50: '#FEFDF8',
          100: '#FEF8E7',
          200: '#FDEEC8',
          300: '#FBDF9A',
          400: '#F7C960',
          500: '#E8A838',   // Main warning - sophisticated amber
          600: '#D18B1F',
          700: '#B06F1A',
          800: '#8D581B',
          900: '#744919',
          950: '#4A2B0B',
        },

        // Coral - Errors and dangers (soft, professional)
        coral: {
          50: '#FFF5F5',
          100: '#FFEBEB',
          200: '#FFD6D6',
          300: '#FFB3B3',
          400: '#FF8585',
          500: '#F56565',   // Main error - soft coral red
          600: '#E84855',
          700: '#D13742',
          800: '#B02937',
          900: '#922330',
          950: '#5C1419',
        },

        // Stone - Neutral scale with subtle warmth
        stone: {
          50: '#FAFAF9',
          100: '#F5F5F4',
          200: '#E7E5E4',
          300: '#D6D3D1',
          400: '#A8A29E',
          500: '#78716C',
          600: '#57534E',
          700: '#44403C',
          800: '#292524',
          900: '#1C1917',
          950: '#0C0A09',
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
