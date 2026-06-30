/**
 * Backward-compatible re-export of the redesigned Meetings page.
 *
 * Skills.tsx now imports and renders `MeetingsPage` directly; this shim
 * keeps existing test mocks and any other importers working without
 * requiring a search-and-replace across the codebase.
 *
 * New code should import from `components/meetings/MeetingsPage` directly.
 */
export { default } from '../meetings/MeetingsPage';
