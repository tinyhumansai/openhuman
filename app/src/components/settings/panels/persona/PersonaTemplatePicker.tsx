import { useT } from '../../../../lib/i18n/I18nContext';
import { applyTemplate, PERSONA_TEMPLATES } from './personaTemplates';

interface PersonaTemplatePickerProps {
  /** Current raw SOUL.md text a template is spliced into. */
  value: string;
  /** Emit the updated SOUL.md text after a template is applied. */
  onChange: (nextSoul: string) => void;
  disabled?: boolean;
}

/**
 * Role starting-points for the guided persona builder (issue #4253, PR2).
 *
 * Applying a template fills the Personality and Communication-style fields for a
 * common role (doctor, researcher, executive, teacher, student, family) and
 * leaves the rest of SOUL.md — including the user-specific "About you" — intact.
 * Nothing is persisted until the user saves, so this is a safe starting point.
 */
const PersonaTemplatePicker = ({
  value,
  onChange,
  disabled = false,
}: PersonaTemplatePickerProps) => {
  const { t } = useT();

  return (
    <div className="space-y-2">
      <div>
        <p className="text-sm font-medium text-content">
          {t('settings.persona.templates.heading')}
        </p>
        <p className="text-xs text-content-muted leading-relaxed">
          {t('settings.persona.templates.desc')}
        </p>
      </div>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {PERSONA_TEMPLATES.map(template => (
          <button
            key={template.id}
            type="button"
            disabled={disabled}
            data-testid={`persona-template-${template.id}`}
            onClick={() => onChange(applyTemplate(value, template))}
            className="flex flex-col items-start gap-0.5 rounded-lg border border-line-strong bg-surface px-3 py-2 text-left transition-colors hover:border-primary-400 hover:bg-surface-hover disabled:opacity-50">
            <span className="text-sm font-medium text-content">{t(template.labelKey)}</span>
            <span className="text-[11px] text-content-muted leading-snug">
              {t(template.descriptionKey)}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
};

export default PersonaTemplatePicker;
