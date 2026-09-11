# intelligence/ — requests to whoever owns i18n keys / ui/index.ts barrel

No new i18n keys were needed — every string used an existing t() key or a
literal that was already there before migration.

No new barrel exports were needed — every primitive used (Button, Dialog,
AlertDialog, Switch, TextField, TextArea, NativeSelect, Checkbox, Table*,
Field, Label, Slider) was already exported from app/src/components/ui/index.ts.

Nothing outstanding from this bucket.
