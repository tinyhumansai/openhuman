/**
 * Moved to `components/ui/Card`. This shim keeps the ~45 existing call sites
 * compiling while they migrate to the shared name; delete it once
 * `rg "settings/controls"` comes back empty.
 */
export { default, type CardProps as SettingsSectionProps } from '../../ui/Card';
