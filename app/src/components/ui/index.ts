/**
 * The UI primitive layer — the only sanctioned import path for shared controls.
 *
 * This barrel previously omitted `Button` and `Input`, which is the likeliest
 * reason 491 raw `<button>` elements grew alongside 591 `<Button>` ones: the
 * barrel did not offer one. Everything is exported here now; if a primitive
 * exists, import it from `components/ui` rather than reaching into the file.
 *
 * WHAT IS NOT HERE, AND WHY. `ButtonGroup` and `HoverCard` were built, tested,
 * exported and then imported by nothing. That state is worse than absence: a
 * primitive with no consumers reads as "already migrated" in every audit while
 * the hand-rolled markup it was meant to replace stays in place, and its
 * passing tests give false confidence about code no user reaches. Both were
 * checked against this app's real surfaces and deleted rather than
 * force-fitted:
 *
 * - `ButtonGroup` — a segmented row of joined buttons. An app-wide search for
 *   the markup it replaces (`rounded-l-none`, `rounded-r-none`,
 *   `first:rounded-l`, `last:rounded-r`, `-ml-px`) returns ZERO hits outside
 *   this directory. The one near-miss, `settings/panels/NotificationRoutingPanel`,
 *   is a `grid grid-cols-3 divide-x` stat readout — three text pairs, no
 *   buttons. The segmented-SELECTION case it might otherwise serve already
 *   belongs to `ToggleGroup`, which has real consumers.
 *
 * - `HoverCard` — a rich preview surface opening on hover and focus. The three
 *   `group-hover:` panels it was meant to replace are not hover cards:
 *   `chat/SuperContextToggle` is `role="tooltip"`, a label for a control, which
 *   is `Tooltip`'s job (HoverCard's own doc comment said "NOT a tooltip");
 *   `skills/SkillsExplorerTab` is a reveal-on-hover Disconnect BUTTON, not a
 *   preview; and `intelligence/MemoryGraph` hovers an SVG `<circle>` inside a
 *   pan/zoom transform and renders a DOCKED status bar at the panel's foot, not
 *   an anchored floating panel — Radix HoverCard anchors to a trigger element,
 *   so adopting it there would change the layout, not just the implementation.
 *
 * Do not re-port either without a concrete consumer to adopt it in the same
 * change.
 */

// Actions
export { default as Button, buttonVariants, type ButtonProps } from './Button';

// Form controls
export { default as Input, type InputProps } from './Input';
export {
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupRoot,
  type InputGroupAddonProps,
  type InputGroupButtonProps,
  type InputGroupInputProps,
  type InputGroupProps,
} from './InputGroup';
export { default as TextField, type TextFieldProps } from './TextField';
export { default as TextArea, type TextAreaProps } from './TextArea';
export { default as NumberField, type NumberFieldProps } from './NumberField';
export { default as NativeSelect, type NativeSelectProps } from './NativeSelect';
export { default as Checkbox, type CheckboxProps } from './Checkbox';
export { default as Switch, type SwitchProps } from './Switch';
export { default as Label, type LabelProps } from './Label';
export { default as Field, type FieldProps } from './Field';
export {
  RadioGroupItem,
  RadioGroupRoot,
  radioGroupItemVariants,
  type RadioGroupItemProps,
  type RadioGroupRootProps,
} from './RadioGroup';
export { default as Toggle, toggleVariants, type ToggleProps } from './Toggle';
export {
  ToggleGroupItem,
  ToggleGroupRoot,
  type ToggleGroupItemProps,
  type ToggleGroupProps,
} from './ToggleGroup';
export {
  default as Slider,
  sliderThumbVariants,
  sliderTrackVariants,
  type SliderProps,
  type SliderSize,
} from './Slider';
export {
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectRoot,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
  type SelectContentProps,
  type SelectSize,
  type SelectTriggerProps,
} from './Select';

// Surfaces & content
export { default as Card, type CardProps } from './Card';
export {
  Alert,
  AlertDescription,
  AlertTitle,
  alertVariants,
  type AlertDensity,
  type AlertProps,
  type AlertVariant,
} from './Alert';
export { default as Badge, badgeVariants, type BadgeProps, type BadgeVariant } from './Badge';
export { default as Separator, type SeparatorProps } from './Separator';
export { default as EmptyState, type EmptyStateProps } from './EmptyState';
export { default as StatusLine, type StatusLineProps } from './StatusLine';
export { default as ListRow, type ListRowProps } from './ListRow';
export { default as Progress, type ProgressProps } from './Progress';
export { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from './Table';
export {
  default as DataTable,
  type DataTableColumn,
  type DataTableFilter,
  type DataTableProps,
  type DataTableSearch,
} from './DataTable';
export { AvatarFallback, AvatarImage, AvatarRoot } from './Avatar';
export {
  AccordionContent,
  AccordionItem,
  AccordionRoot,
  AccordionTrigger,
  accordionContentVariants,
  accordionItemVariants,
  accordionTriggerVariants,
  accordionVariants,
  type AccordionContentProps,
  type AccordionItemProps,
  type AccordionRootProps,
  type AccordionSize,
  type AccordionTriggerProps,
  type AccordionVariant,
} from './Accordion';
export {
  CollapsibleContent,
  CollapsibleRoot,
  CollapsibleTrigger,
  collapsibleContentVariants,
  collapsibleTriggerVariants,
  collapsibleVariants,
  type CollapsibleContentProps,
  type CollapsibleRootProps,
  type CollapsibleSize,
  type CollapsibleTriggerProps,
  type CollapsibleVariant,
} from './Collapsible';

// Overlays
export {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogRoot,
  DialogTitle,
  DialogTrigger,
  type DialogContentProps,
} from './Dialog';
export {
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogOverlay,
  AlertDialogRoot,
  AlertDialogTitle,
  AlertDialogTrigger,
  type AlertDialogActionProps,
  type AlertDialogContentProps,
  type AlertDialogOverlayProps,
} from './AlertDialog';
export {
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetRoot,
  SheetTitle,
  SheetTrigger,
  sheetVariants,
  type SheetContentProps,
} from './Sheet';
export {
  PopoverAnchor,
  PopoverClose,
  PopoverContent,
  PopoverRoot,
  PopoverTrigger,
  type PopoverContentProps,
} from './Popover';
export {
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  type DropdownMenuContentProps,
} from './DropdownMenu';
export { TabsContent, TabsList, TabsRoot, TabsTrigger } from './Tabs';

export { default as Tooltip } from './Tooltip';
export { ModalShell, type ModalClosePolicy, type ModalShellProps } from './ModalShell';
export { ConfirmDialog, type ConfirmDialogProps } from './ConfirmDialog';

// Navigation & layout
export {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_ICON_WIDTH,
  SIDEBAR_KEYBOARD_STEP,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuIcon,
  SidebarMenuItem,
  SidebarMenuLabel,
  SidebarProvider,
  SidebarRail,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
  type SidebarCollapsible,
  type SidebarContextValue,
  type SidebarGroupLabelProps,
  type SidebarInsetProps,
  type SidebarMenuBadgeProps,
  type SidebarMenuButtonProps,
  type SidebarMenuButtonSize,
  type SidebarProps,
  type SidebarProviderProps,
  type SidebarRailProps,
  type SidebarSide,
  type SidebarState,
  type SidebarTriggerProps,
} from './Sidebar';

// Feedback & misc
export { Spinner, CheckIcon, CloseIcon, WarningIcon } from './icons';
export { CenteredLoadingState, ErrorBanner, InlineLoadingStatus } from './LoadingState';
export { default as BetaBanner } from './BetaBanner';
export { default as BetaIndicator } from './BetaIndicator';
export { default as VisuallyHidden, type VisuallyHiddenProps } from './VisuallyHidden';
