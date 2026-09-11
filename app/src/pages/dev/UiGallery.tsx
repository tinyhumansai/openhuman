import { useState } from 'react';

import { Source, Sources, SourcesContent, SourcesTrigger } from '../../components/ai-elements';
import {
  AccordionContent,
  AccordionItem,
  AccordionRoot,
  AccordionTrigger,
  Alert,
  AlertDescription,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogRoot,
  AlertDialogTitle,
  AlertDialogTrigger,
  AlertTitle,
  AvatarFallback,
  AvatarImage,
  AvatarRoot,
  Badge,
  Button,
  Card,
  Checkbox,
  CollapsibleContent,
  CollapsibleRoot,
  CollapsibleTrigger,
  ConfirmDialog,
  DialogContent,
  DialogRoot,
  DialogTitle,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
  EmptyState,
  Field,
  Input,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupRoot,
  Label,
  ListRow,
  NativeSelect,
  NumberField,
  PopoverContent,
  PopoverRoot,
  PopoverTrigger,
  Progress,
  RadioGroupItem,
  RadioGroupRoot,
  SelectContent,
  SelectItem,
  SelectRoot,
  SelectTrigger,
  SelectValue,
  Separator,
  SheetContent,
  SheetRoot,
  SheetTitle,
  SheetTrigger,
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
  Slider,
  StatusLine,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TabsContent,
  TabsList,
  TabsRoot,
  TabsTrigger,
  TextArea,
  TextField,
  Toggle,
  ToggleGroupItem,
  ToggleGroupRoot,
  Tooltip,
  VisuallyHidden,
} from '../../components/ui';

/**
 * Dev-only gallery of every shared UI primitive, reachable at `#/dev/ui`.
 *
 * Three jobs, only the first of which is obvious:
 *  1. a visual review surface for the primitive layer, in whichever theme the
 *     app is currently set to — switch themes in Settings and reload here;
 *  2. it *imports* every primitive, so `knip` does not report the new exports
 *     as unused during the window where feature code has not adopted them yet;
 *  3. somewhere for the a11y smoke lane to grow against real controls.
 *
 * Not linked from any nav. `src/pages/dev/**` is coverage-excluded by design.
 */
const BUTTON_VARIANTS = ['primary', 'secondary', 'tertiary'] as const;
const BUTTON_SIZES = ['xs', 'sm', 'md', 'lg', 'xl'] as const;
const BADGE_VARIANTS = ['neutral', 'primary', 'success', 'warning', 'danger'] as const;

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <h2 className="text-sm font-semibold text-content">{title}</h2>
      <Card>
        <div className="space-y-4 p-4">{children}</div>
      </Card>
    </section>
  );
}

export default function UiGallery() {
  const [checked, setChecked] = useState(true);
  const [indeterminate, setIndeterminate] = useState(true);
  const [switched, setSwitched] = useState(true);
  const [numeric, setNumeric] = useState('30');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [alertOpen, setAlertOpen] = useState(false);
  const [radio, setRadio] = useState('comfortable');
  const [pressed, setPressed] = useState(false);
  const [alignment, setAlignment] = useState('left');
  const [volume, setVolume] = useState([40]);
  const [range, setRange] = useState([20, 70]);
  const [fruit, setFruit] = useState('apple');
  const [sidebarOpen, setSidebarOpen] = useState(true);

  return (
    <div className="mx-auto max-w-4xl space-y-8 p-6">
      <header className="space-y-1">
        <h1 className="text-xl font-semibold text-content">UI primitives</h1>
        <p className="text-sm text-content-muted">
          Every shared primitive, in the active theme. Radix supplies behaviour; the styling is this
          app&apos;s semantic tokens, so each control follows a custom theme.
        </p>
      </header>

      <Section title="Button — variant x tone">
        {BUTTON_VARIANTS.map(variant => (
          <div key={variant} className="flex flex-wrap items-center gap-2">
            <span className="w-20 text-xs text-content-muted">{variant}</span>
            <Button variant={variant}>Default</Button>
            <Button variant={variant} tone="danger">
              Danger
            </Button>
            <Button variant={variant} disabled>
              Disabled
            </Button>
          </div>
        ))}
        <Separator />
        <div className="flex flex-wrap items-center gap-2">
          {BUTTON_SIZES.map(size => (
            <Button key={size} size={size}>
              {size}
            </Button>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {BUTTON_SIZES.map(size => (
            <Button key={size} size={size} iconOnly aria-label={`icon ${size}`} variant="secondary">
              +
            </Button>
          ))}
        </div>
      </Section>

      <Section title="Badge">
        <div className="flex flex-wrap items-center gap-2">
          {BADGE_VARIANTS.map(variant => (
            <Badge key={variant} variant={variant}>
              {variant}
            </Badge>
          ))}
        </div>
      </Section>

      <Section title="Form controls">
        <Field
          htmlFor="gallery-text"
          label="Text field"
          description="A labelled row."
          control={<TextField id="gallery-text" placeholder="Placeholder" />}
        />
        <Field
          htmlFor="gallery-mono"
          label="Monospace"
          control={<TextField id="gallery-mono" mono defaultValue="sk-abc123" />}
        />
        <Field
          htmlFor="gallery-invalid"
          label="Invalid"
          control={<Input id="gallery-invalid" invalid defaultValue="nope" />}
        />
        <Field
          htmlFor="gallery-area"
          label="Text area"
          stacked
          control={<TextArea id="gallery-area" rows={3} placeholder="Longer text" />}
        />
        <Field
          htmlFor="gallery-select"
          label="Native select"
          control={
            <NativeSelect id="gallery-select" defaultValue="b">
              <option value="a">Alpha</option>
              <option value="b">Beta</option>
            </NativeSelect>
          }
        />
        <Field
          htmlFor="gallery-switch"
          label="Switch"
          control={<Switch id="gallery-switch" checked={switched} onCheckedChange={setSwitched} />}
        />
        <Field
          htmlFor="gallery-check"
          label="Checkbox"
          control={
            <div className="flex items-center gap-3">
              <Checkbox
                id="gallery-check"
                checked={checked}
                onCheckedChange={setChecked}
                aria-label="Checkbox"
              />
              <Checkbox
                checked={false}
                indeterminate={indeterminate}
                onCheckedChange={() => setIndeterminate(false)}
                aria-label="Indeterminate"
              />
            </div>
          }
        />
        <Field
          htmlFor="gallery-number"
          label="Number field"
          control={
            <NumberField
              id="gallery-number"
              value={numeric}
              onChange={setNumeric}
              onCommit={() => {}}
              unit="seconds"
              min={1}
              max={120}
              aria-label="Timeout"
            />
          }
        />
        <Field label="Disabled row" disabled control={<Label>Not interactive</Label>} />
      </Section>

      <Section title="Overlays">
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={() => setDialogOpen(true)}>Open dialog</Button>
          <Button variant="secondary" tone="danger" onClick={() => setConfirmOpen(true)}>
            Open confirm
          </Button>

          <SheetRoot>
            <SheetTrigger asChild>
              <Button variant="secondary">Open sheet</Button>
            </SheetTrigger>
            <SheetContent side="right" className="p-5">
              <SheetTitle className="text-sm font-semibold text-content">Sheet</SheetTitle>
              <p className="mt-2 text-sm text-content-muted">Anchored to an edge, focus trapped.</p>
            </SheetContent>
          </SheetRoot>

          <PopoverRoot>
            <PopoverTrigger asChild>
              <Button variant="tertiary">Popover</Button>
            </PopoverTrigger>
            <PopoverContent>Anchored, dismissable, collision-aware.</PopoverContent>
          </PopoverRoot>

          <DropdownMenuRoot>
            <DropdownMenuTrigger asChild>
              <Button variant="tertiary">Menu</Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              <DropdownMenuItem>Rename</DropdownMenuItem>
              <DropdownMenuItem>Duplicate</DropdownMenuItem>
              <DropdownMenuItem disabled>Disabled</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenuRoot>

          <Tooltip label="A tooltip">
            <Button variant="tertiary">Hover me</Button>
          </Tooltip>
        </div>

        {dialogOpen && (
          <DialogRoot open onOpenChange={next => !next && setDialogOpen(false)}>
            <DialogContent className="p-5">
              <DialogTitle className="text-sm font-semibold text-content">Dialog</DialogTitle>
              <p className="mt-2 text-sm text-content-muted">
                Escape, outside click and focus trap all come from Radix.
              </p>
              <div className="mt-4 flex justify-end">
                <Button size="sm" onClick={() => setDialogOpen(false)}>
                  Close
                </Button>
              </div>
            </DialogContent>
          </DialogRoot>
        )}

        {confirmOpen && (
          <ConfirmDialog
            title="Delete this?"
            body="Labels default through i18n, so this reads in the active locale."
            destructive
            onConfirm={() => setConfirmOpen(false)}
            onCancel={() => setConfirmOpen(false)}
          />
        )}
      </Section>

      <Section title="Tabs">
        <TabsRoot defaultValue="one">
          <TabsList>
            <TabsTrigger value="one">Overview</TabsTrigger>
            <TabsTrigger value="two">Activity</TabsTrigger>
            <TabsTrigger value="three">Settings</TabsTrigger>
          </TabsList>
          <TabsContent value="one" className="pt-3 text-sm text-content-muted">
            Arrow keys move between tabs; only one tab is a tab stop.
          </TabsContent>
          <TabsContent value="two" className="pt-3 text-sm text-content-muted">
            Activity panel.
          </TabsContent>
          <TabsContent value="three" className="pt-3 text-sm text-content-muted">
            Settings panel.
          </TabsContent>
        </TabsRoot>
      </Section>

      <Section title="Data">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Status</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow>
              <TableCell>Nightly sync</TableCell>
              <TableCell>
                <Badge variant="success">Healthy</Badge>
              </TableCell>
            </TableRow>
            <TableRow>
              <TableCell>Index rebuild</TableCell>
              <TableCell>
                <Badge variant="warning">Degraded</Badge>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <Separator />
        <ul className="rounded-lg border border-line">
          <ListRow label="allow-list-entry.example.com" removeLabel="Remove" onRemove={() => {}} />
          <ListRow
            label="/usr/local/bin/tool"
            mono
            removeLabel="Remove"
            badge={<Badge variant="neutral">path</Badge>}
            onRemove={() => {}}
          />
        </ul>
        <EmptyState label="Nothing here yet." />
      </Section>

      <Section title="Radio group">
        <RadioGroupRoot value={radio} onValueChange={setRadio} aria-label="Density">
          {['compact', 'comfortable', 'spacious'].map(value => (
            <label key={value} className="flex items-center gap-2 text-sm text-content">
              <RadioGroupItem value={value} id={`gallery-radio-${value}`} />
              <span>{value}</span>
            </label>
          ))}
        </RadioGroupRoot>
        <Separator />
        <RadioGroupRoot className="flex-row gap-4" defaultValue="sm" aria-label="Sizes">
          <RadioGroupItem value="sm" size="sm" aria-label="Small" />
          <RadioGroupItem value="md" size="md" aria-label="Medium" />
          <RadioGroupItem value="lg" size="lg" aria-label="Large" />
        </RadioGroupRoot>
      </Section>

      <Section title="Select">
        <SelectRoot value={fruit} onValueChange={setFruit}>
          <SelectTrigger className="w-56" aria-label="Fruit">
            <SelectValue placeholder="Pick one" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="apple">Apple</SelectItem>
            <SelectItem value="banana">Banana</SelectItem>
            <SelectItem value="cherry">Cherry</SelectItem>
            <SelectItem value="durian" disabled>
              Durian (disabled)
            </SelectItem>
          </SelectContent>
        </SelectRoot>
        <SelectRoot defaultValue="apple">
          <SelectTrigger inputSize="sm" className="w-56" aria-label="Fruit, small">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="apple">Apple</SelectItem>
            <SelectItem value="banana">Banana</SelectItem>
          </SelectContent>
        </SelectRoot>
      </Section>

      <Section title="Toggle & toggle group">
        <div className="flex flex-wrap items-center gap-2">
          <Toggle pressed={pressed} onPressedChange={setPressed}>
            Bold
          </Toggle>
          <Toggle variant="secondary" defaultPressed>
            Secondary
          </Toggle>
          <Toggle tone="danger">Danger</Toggle>
          <Toggle iconOnly aria-label="Star">
            ★
          </Toggle>
          <Toggle disabled>Disabled</Toggle>
        </div>
        <Separator />
        <ToggleGroupRoot
          type="single"
          value={alignment}
          onValueChange={next => next && setAlignment(next)}
          aria-label="Alignment">
          <ToggleGroupItem value="left">Left</ToggleGroupItem>
          <ToggleGroupItem value="center">Center</ToggleGroupItem>
          <ToggleGroupItem value="right">Right</ToggleGroupItem>
        </ToggleGroupRoot>
        <ToggleGroupRoot type="multiple" variant="secondary" size="sm" aria-label="Text style">
          <ToggleGroupItem value="bold">B</ToggleGroupItem>
          <ToggleGroupItem value="italic">I</ToggleGroupItem>
          <ToggleGroupItem value="underline">U</ToggleGroupItem>
        </ToggleGroupRoot>
      </Section>

      <Section title="Slider">
        <Slider
          value={volume}
          onValueChange={setVolume}
          max={100}
          step={1}
          thumbLabels={['Volume']}
        />
        <p className="text-xs text-content-muted">Value: {volume[0]}</p>
        <Slider
          size="sm"
          value={range}
          onValueChange={setRange}
          max={100}
          thumbLabels={['Minimum', 'Maximum']}
        />
        <p className="text-xs text-content-muted">
          Range: {range[0]}–{range[1]}
        </p>
        <Slider defaultValue={[60]} disabled thumbLabels={['Disabled']} />
      </Section>

      <Section title="Avatar">
        <div className="flex flex-wrap items-center gap-3">
          <AvatarRoot className="h-6 w-6">
            <AvatarFallback>ab</AvatarFallback>
          </AvatarRoot>
          <AvatarRoot>
            <AvatarFallback>cd</AvatarFallback>
          </AvatarRoot>
          <AvatarRoot className="h-12 w-12">
            <AvatarImage src="https://example.invalid/missing.png" alt="" />
            <AvatarFallback delayMs={0}>ef</AvatarFallback>
          </AvatarRoot>
        </div>
        <p className="text-xs text-content-muted">
          The image above never resolves, so the fallback is what renders — that is the point of the
          delayed swap.
        </p>
      </Section>

      <Section title="Accordion">
        <AccordionRoot type="single" collapsible defaultValue="a">
          <AccordionItem value="a">
            <AccordionTrigger>Plain, first row</AccordionTrigger>
            <AccordionContent>Hairlines between rows, no container chrome.</AccordionContent>
          </AccordionItem>
          <AccordionItem value="b">
            <AccordionTrigger>Plain, second row</AccordionTrigger>
            <AccordionContent>Only one row opens at a time.</AccordionContent>
          </AccordionItem>
        </AccordionRoot>
        <Separator />
        <AccordionRoot type="multiple" variant="contained">
          <AccordionItem value="a" variant="contained">
            <AccordionTrigger size="sm">Contained, small</AccordionTrigger>
            <AccordionContent size="sm">One bordered container.</AccordionContent>
          </AccordionItem>
          <AccordionItem value="b" variant="contained">
            <AccordionTrigger size="sm">Both can be open</AccordionTrigger>
            <AccordionContent size="sm">Multiple-type root.</AccordionContent>
          </AccordionItem>
        </AccordionRoot>
        <Separator />
        <AccordionRoot type="single" collapsible variant="card">
          <AccordionItem value="a" variant="card">
            <AccordionTrigger size="lg">Card, large</AccordionTrigger>
            <AccordionContent size="lg">Each item is its own card.</AccordionContent>
          </AccordionItem>
        </AccordionRoot>
      </Section>

      <Section title="Collapsible">
        <CollapsibleRoot variant="card" defaultOpen>
          <CollapsibleTrigger>Advanced settings</CollapsibleTrigger>
          <CollapsibleContent>
            One trigger, one region — no roving between siblings the way an accordion has.
          </CollapsibleContent>
        </CollapsibleRoot>
        <CollapsibleRoot>
          <CollapsibleTrigger size="sm">Plain, small</CollapsibleTrigger>
          <CollapsibleContent size="sm">No container chrome.</CollapsibleContent>
        </CollapsibleRoot>
      </Section>

      <Section title="Alert dialog">
        <div className="flex flex-wrap items-center gap-2">
          <AlertDialogRoot open={alertOpen} onOpenChange={setAlertOpen}>
            <AlertDialogTrigger asChild>
              <Button variant="secondary" tone="danger">
                Open alert dialog
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogTitle className="text-sm font-semibold text-content">
                Delete this workspace?
              </AlertDialogTitle>
              <AlertDialogDescription className="mt-2 text-sm text-content-muted">
                Unlike a dialog, this one has no outside-click dismissal and focuses cancel on open.
              </AlertDialogDescription>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction>Delete</AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialogRoot>

          <AlertDialogRoot>
            <AlertDialogTrigger asChild>
              <Button variant="tertiary">Benign tone</Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogTitle className="text-sm font-semibold text-content">
                Apply the update?
              </AlertDialogTitle>
              <AlertDialogDescription className="mt-2 text-sm text-content-muted">
                Pass tone=&quot;default&quot; when the decision is not destructive.
              </AlertDialogDescription>
              <AlertDialogFooter>
                <AlertDialogCancel>Not now</AlertDialogCancel>
                <AlertDialogAction tone="default">Apply</AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialogRoot>
        </div>
      </Section>

      <Section title="Visually hidden">
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="secondary">
            ×<VisuallyHidden>Close the panel</VisuallyHidden>
          </Button>
          <span className="text-xs text-content-muted">
            The button above reads as “Close the panel” to a screen reader and as “×” on screen —
            nothing renders here, by design.
          </span>
        </div>
      </Section>

      <Section title="Alert">
        {(['default', 'info', 'success', 'warning', 'destructive'] as const).map(variant => (
          <Alert key={variant} variant={variant}>
            <div>
              <AlertTitle>{variant}</AlertTitle>
              <AlertDescription>
                A short explanation of what happened and what to do next.
              </AlertDescription>
            </div>
          </Alert>
        ))}
        <p className="text-xs text-content-muted">
          The destructive and warning variants carry role=&quot;alert&quot;; the quieter ones do
          not, so a static page of them does not shout at a screen reader.
        </p>
      </Section>

      <Section title="Input group">
        <InputGroupRoot>
          <InputGroupAddon>https://</InputGroupAddon>
          <InputGroupInput placeholder="example.com" aria-label="Domain" />
          <InputGroupButton>Check</InputGroupButton>
        </InputGroupRoot>
        <InputGroupRoot size="sm">
          <InputGroupInput placeholder="Timeout" aria-label="Timeout" />
          <InputGroupAddon>seconds</InputGroupAddon>
        </InputGroupRoot>
        <InputGroupRoot size="lg">
          <InputGroupAddon aria-hidden>🔍</InputGroupAddon>
          <InputGroupInput placeholder="Search" aria-label="Search" />
        </InputGroupRoot>
      </Section>

      <Section title="Sidebar">
        <SidebarProvider
          open={sidebarOpen}
          onOpenChange={setSidebarOpen}
          className="h-64 overflow-hidden rounded-lg border border-line bg-surface-canvas">
          <Sidebar collapsible="icon" className="bg-surface-muted">
            <SidebarHeader>
              <SidebarMenuLabel>Workspace</SidebarMenuLabel>
            </SidebarHeader>
            <SidebarSeparator />
            <SidebarContent>
              <SidebarGroup>
                <SidebarGroupLabel>Navigation</SidebarGroupLabel>
                <SidebarGroupContent>
                  <SidebarMenu>
                    <SidebarMenuItem>
                      <SidebarMenuButton isActive>
                        <SidebarMenuIcon aria-hidden>◆</SidebarMenuIcon>
                        <SidebarMenuLabel>Chat</SidebarMenuLabel>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                    <SidebarMenuItem>
                      <SidebarMenuButton>
                        <SidebarMenuIcon aria-hidden>◇</SidebarMenuIcon>
                        <SidebarMenuLabel>Notifications</SidebarMenuLabel>
                        <SidebarMenuBadge tone="attention">3</SidebarMenuBadge>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                    <SidebarMenuItem>
                      <SidebarMenuButton size="sm">
                        <SidebarMenuIcon aria-hidden>○</SidebarMenuIcon>
                        <SidebarMenuLabel>Settings</SidebarMenuLabel>
                        <SidebarMenuBadge>12</SidebarMenuBadge>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  </SidebarMenu>
                </SidebarGroupContent>
              </SidebarGroup>
            </SidebarContent>
            <SidebarFooter>
              <SidebarMenuLabel>v0.0.0-dev</SidebarMenuLabel>
            </SidebarFooter>
          </Sidebar>
          <SidebarRail aria-label="Resize the sidebar" />
          <SidebarInset className="p-4">
            <SidebarTrigger
              aria-label="Toggle the sidebar"
              className="rounded-md border border-line px-2 py-1 text-xs text-content-muted">
              Toggle
            </SidebarTrigger>
            <p className="mt-3 text-sm text-content-muted">
              Drag the rail, or focus it and use the arrow keys — width is clamped between the
              exported minimum and maximum. Collapsing narrows to the icon width rather than
              unmounting, because collapsible=&quot;icon&quot; is set here.
            </p>
          </SidebarInset>
        </SidebarProvider>
      </Section>

      <Section title="Feedback">
        <Progress value={42} aria-label="Progress" />
        <StatusLine saving={false} savedNote="Saved" savingLabel="Saving…" />
        <StatusLine saving savingLabel="Saving…" />
        <StatusLine saving={false} error="Could not reach the server." savingLabel="Saving…" />
      </Section>

      {/* A <div>, not a <header>: two banner landmarks in one document is an
          axe violation, and the page already has one at the top. */}
      <div className="space-y-1 pt-4">
        <h1 className="text-xl font-semibold text-content">AI elements</h1>
        <p className="text-sm text-content-muted">
          Chat-surface components adapted from vercel/ai-elements onto these primitives and tokens.
        </p>
      </div>

      <Section title="Sources">
        <Sources defaultOpen>
          <SourcesTrigger count={2} />
          <SourcesContent>
            <Source href="https://example.invalid/a" title="Architecture overview" />
            <Source href="https://example.invalid/b" title="Hardening plan" />
          </SourcesContent>
        </Sources>
      </Section>
    </div>
  );
}
