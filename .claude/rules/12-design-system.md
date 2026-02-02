# Design System - Crypto Community Platform

## Design Philosophy

Our design system is built on **trust**, **usefulness**, and **simplicity** - core principles essential for crypto/fintech applications where users handle sensitive financial discussions and data.

## Typography

### Font Selection - Psychology of Trust

**Primary Font: Inter**

- **Why**: Sans-serif font that signals modernity, efficiency, and clarity
- **Psychology**: Clean, digital-first appearance builds trust in tech platforms
- **Usage**: Body text, UI elements, navigation
- **Weights**: 300, 400, 500, 600, 700

**Monospace Font: JetBrains Mono**

- **Why**: Essential for crypto addresses, transaction hashes, code snippets
- **Psychology**: Monospace fonts convey technical precision and accuracy
- **Usage**: Crypto addresses, prices, technical data, code blocks
- **Weights**: 300, 400, 500, 600

### Font Hierarchy

```css
h1: text-3xl lg:text-4xl (48px desktop, 30px mobile)
h2: text-2xl lg:text-3xl (36px desktop, 24px mobile)
h3: text-xl lg:text-2xl (24px desktop, 20px mobile)
h4: text-lg lg:text-xl (20px desktop, 18px mobile)
Body: text-base (16px) - optimal for readability
Small: text-sm (14px) - secondary information
```

## Color Palette - Trust & Professionalism

### Primary Colors - Ocean Blue (Updated)

- **Primary-500**: `#4A83DD` - Main brand color optimized for dark backgrounds
- **Primary-600**: `#3D6DC4` - Interactive hover states
- **Primary-700**: `#345A9F` - Active states, emphasis

**Psychology**: Deep ocean blue builds trust and sophistication for crypto platforms.

### Success Colors - Sage Green (Updated)

- **Success-500**: `#4DC46F` - Refined success green for growth indicators
- **Success-600**: `#3BA858` - Interactive success states

**Psychology**: Sophisticated sage green represents growth and financial success.

### Warning & Error Colors (Updated)

- **Warning-500**: `#E8A838` - Sophisticated amber for attention states
- **Error-500**: `#F56565` - Soft coral red for professional error handling

### Canvas Colors - Background Layers (Updated)

- **Canvas-50**: `#FAFAF9` - Base background with subtle warmth
- **Canvas-100**: `#F5F5F4` - Secondary background
- **Canvas-150**: `#EDEDEC` - Tertiary background
- **Canvas-200**: `#E5E5E3` - Card background
- **Canvas-300**: `#D4D4D1` - Hover states

### Stone/Slate Neutrals - Text & UI Elements

- **Stone-500**: `#78716C` - Mid-tone text
- **Stone-900**: `#1C1917` - Primary text, high contrast
- **Slate-400**: `#94A3B8` - Secondary text, data elements

### Market Colors - Crypto Trading

- **Bullish**: `#4DC46F` - Green for gains (matches sage)
- **Bearish**: `#F56565` - Red for losses (matches coral)
- **Neutral**: `#94A3B8` - Gray for no change
- **Bitcoin**: `#F7931A` - Bitcoin orange
- **Ethereum**: `#627EEA` - Ethereum purple
- **Stablecoin**: `#5B9BF3` - Blue for stables

### Accent Colors - Special Elements

- **Lavender**: `#9B8AFB` - Premium features
- **Mint**: `#6EE7B7` - Achievements
- **Sky**: `#7DD3FC` - Notifications
- **Rose**: `#FDA4AF` - Alerts
- **Gold**: `#FCD34D` - Rewards

## Component Library

### Buttons - Clear Action Hierarchy

```css
.btn-primary - Main actions (Send, Buy, Confirm)
.btn-secondary - Secondary actions (Cancel, Back)
.btn-success - Positive confirmations (Approve, Accept)
.btn-danger - Destructive actions (Delete, Reject)
```

**Design Principles**:

- Clear visual hierarchy prevents costly mistakes
- Consistent interaction patterns build familiarity
- Adequate touch targets (44px min) for mobile accessibility

### Cards - Content Organization

```css
.card - Basic content container with soft shadows
.card-hover - Interactive cards with elevation feedback
```

**Psychology**: Cards create clear content boundaries, reduce cognitive load, and provide familiar interaction patterns.

### Inputs - Trustworthy Data Entry

```css
.input-primary - Consistent form styling with focus states
```

**Features**:

- Clear focus indicators for accessibility
- Proper contrast ratios (WCAG 2.1 AA compliant)
- Error states with helpful messaging
- Placeholder text that guides without overwhelming

### Status Indicators - Clear Information Hierarchy

```css
.status-online - Connected, active states
.status-offline - Disconnected, inactive states
.status-warning - Attention required
```

### Navigation - Intuitive Wayfinding

```css
.nav-item - Default navigation states
.nav-item-active - Current location indicator
```

## Layout Principles

### Spacing System

- **Base unit**: 4px (0.25rem)
- **Common spacing**: 8px, 16px, 24px, 32px, 48px
- **Component padding**: 24px (6 units)
- **Section spacing**: 48px (12 units)

### Responsive Breakpoints

```css
sm: 640px   - Small tablets
md: 768px   - Tablets
lg: 1024px  - Small desktops
xl: 1280px  - Large desktops
```

### Grid System

- **Mobile**: Single column, 16px margins
- **Tablet**: 2-3 columns, 24px margins
- **Desktop**: Multi-column layouts, 32px margins

## Interactive Elements

### Shadows - Depth & Hierarchy

```css
shadow-soft: Subtle elevation for cards
shadow-medium: Interactive hover states
shadow-strong: Modals, overlays, emphasis
```

### Animation - Smooth Interactions

- **Duration**: 200ms for micro-interactions, 300ms for transitions
- **Easing**: `ease-in-out` for natural feel
- **Principles**: Reduce motion for accessibility, maintain performance

### Focus States - Accessibility First

- **Ring**: 2px blue outline with 2px offset
- **Color**: Primary-500 for consistency
- **Visibility**: Clear on all interactive elements

## Mobile Optimization

### Touch Targets

- **Minimum**: 44px × 44px
- **Recommended**: 48px × 48px for primary actions
- **Spacing**: 8px minimum between interactive elements

### Typography Scale

- **Mobile-first**: Base sizes optimized for readability on small screens
- **Progressive enhancement**: Larger sizes on desktop
- **Line height**: 1.6 for optimal mobile reading

### Safe Areas

- **iOS**: Respect notch and home indicator
- **Android**: Navigation and status bar accommodation
- **CSS**: `env(safe-area-inset-*)` for dynamic adjustment

## Accessibility Standards

### WCAG 2.1 AA Compliance

- **Contrast ratios**: 4.5:1 for normal text, 3:1 for large text
- **Color**: Never sole indicator of information
- **Focus**: Visible focus indicators on all interactive elements
- **Motion**: Respect `prefers-reduced-motion`

### Screen Reader Support

- **Semantic HTML**: Proper heading hierarchy, landmarks
- **ARIA labels**: Descriptive labels for complex interactions
- **Live regions**: Dynamic content announcements

## Usage Guidelines

### When to Use Primary Blue

- **Call-to-action buttons**: Sign up, log in, send transaction
- **Active navigation**: Current page indicators
- **Links**: Primary navigation and important links
- **Progress indicators**: Loading states, completion

### When to Use Success Green

- **Positive confirmations**: Transaction successful, account verified
- **Profit indicators**: Price increases, portfolio gains
- **Status indicators**: Online, connected, active

### When to Use Warning Orange

- **Caution states**: Pending transactions, rate limits
- **Important notices**: Security warnings, updates required
- **Validation**: Form warnings that aren't errors

### When to Use Error Red

- **Destructive actions**: Delete account, remove funds
- **Error states**: Failed transactions, connection errors
- **Loss indicators**: Price decreases, portfolio losses

## Implementation Notes

### CSS Custom Properties

All colors, spacing, and typography scales are available as CSS custom properties for consistent theming.

### Dark Mode Readiness

Color palette includes dark mode variations for future implementation.

### Performance

- **Font loading**: `font-display: swap` for improved loading experience
- **Critical CSS**: Base styles inlined, components loaded asynchronously
- **Animation**: Hardware-accelerated where appropriate

---

_This design system prioritizes user trust through consistent, accessible, and professional visual design - essential for crypto community platforms where financial decisions are made._
