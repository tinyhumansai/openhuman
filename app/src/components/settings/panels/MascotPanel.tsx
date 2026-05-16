import { getMascotPalette, type MascotColor } from '../../../features/human/Mascot/mascotPalette';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  DEFAULT_MASCOT_COLOR,
  selectMascotColor,
  setMascotColor,
  SUPPORTED_MASCOT_COLORS,
} from '../../../store/mascotSlice';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface ColorOption {
  id: MascotColor;
  label: string;
}

const COLOR_OPTIONS: ColorOption[] = [
  { id: 'yellow', label: 'Yellow' },
  { id: 'burgundy', label: 'Burgundy' },
  { id: 'black', label: 'Black' },
  { id: 'navy', label: 'Navy' },
  { id: 'green', label: 'Green' },
];

const MascotPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const storedColor = useAppSelector(selectMascotColor);

  // Filter the menu to colors the asset pipeline currently supports — guards
  // against an older persisted value pointing at a variant a future build
  // removed. The selected swatch still highlights iff the stored color is
  // present; otherwise we silently fall back to the default for the preview.
  const available = COLOR_OPTIONS.filter(opt =>
    (SUPPORTED_MASCOT_COLORS as readonly string[]).includes(opt.id)
  );
  const activeColor: MascotColor = (SUPPORTED_MASCOT_COLORS as readonly string[]).includes(
    storedColor
  )
    ? storedColor
    : DEFAULT_MASCOT_COLOR;

  const handleSelect = (color: MascotColor) => {
    if (color === storedColor) return;
    dispatch(setMascotColor(color));
  };

  return (
    <div>
      <SettingsHeader
        title="Mascot"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-2 px-1">
            Color
          </h3>
          <div className="bg-white rounded-xl border border-stone-200 overflow-hidden">
            {available.length === 0 ? (
              <p className="p-4 text-sm text-stone-500">
                No mascot color variants are available in this build.
              </p>
            ) : (
              <div
                className="grid grid-cols-5 gap-3 p-4"
                role="radiogroup"
                aria-label="Mascot color">
                {available.map(opt => {
                  const palette = getMascotPalette(opt.id);
                  const selected = opt.id === activeColor;
                  return (
                    <button
                      key={opt.id}
                      type="button"
                      role="radio"
                      aria-checked={selected}
                      aria-label={opt.label}
                      onClick={() => handleSelect(opt.id)}
                      data-testid={`mascot-color-${opt.id}`}
                      className={`flex flex-col items-center gap-2 rounded-lg p-2 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 ${
                        selected ? 'bg-stone-100' : 'hover:bg-stone-50'
                      }`}>
                      <span
                        className={`w-10 h-10 rounded-full border-2 transition-shadow ${
                          selected ? 'border-primary-500 shadow-soft' : 'border-stone-200'
                        }`}
                        style={{ backgroundColor: palette.bodyFill }}
                      />
                      <span className="text-xs text-stone-700">{opt.label}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          <p className="text-xs text-stone-500 leading-relaxed px-1 mt-2">
            The selected color is applied everywhere the mascot appears and is remembered across
            restarts.
          </p>
        </div>
      </div>
    </div>
  );
};

export default MascotPanel;
