import { describe, expect, it } from 'vitest';

import ar from '../ar';
import bn from '../bn';
import de from '../de';
import en from '../en';
import es from '../es';
import fr from '../fr';
import hi from '../hi';
import id from '../id';
import itIT from '../it';
import ko from '../ko';
import pl from '../pl';
import pt from '../pt';
import ru from '../ru';
import zhCN from '../zh-CN';

const LOCALES: Record<string, Record<string, string>> = {
  ar,
  bn,
  de,
  en,
  es,
  fr,
  hi,
  id,
  it: itIT,
  ko,
  pl,
  pt,
  ru,
  'zh-CN': zhCN,
};

/**
 * The dismiss dialog is the only place that tells a user how to get the mascot
 * back, so the menu path it quotes has to match what is actually on screen in
 * that language.
 *
 * This is not hypothetical: the first version of these strings was translated
 * from the English path rather than assembled from each locale's own labels,
 * which sent German users to "Darstellung" (the real menu says "Aussehen"),
 * Hindi users to "रूप" (really "दिखावट"), Korean users to "화면" (really "외관"),
 * and several others to a Chat section that does not use that word. A dismissed
 * mascot with a wrong path back is a one-way door.
 */
describe('mascot dismiss dialog — settings path', () => {
  it.each(Object.keys(LOCALES))('%s quotes labels the UI actually shows', locale => {
    const t = LOCALES[locale];
    const body = t['chat.mascot.dismissBody'];
    expect(body, `${locale} is missing chat.mascot.dismissBody`).toBeTruthy();

    for (const key of [
      'nav.settings',
      'settings.appearance.title',
      'settings.appearance.chatHeading',
    ]) {
      const label = t[key];
      expect(label, `${locale} is missing ${key}`).toBeTruthy();
      expect(
        body,
        `${locale}: dismissBody must quote the real ${key} ("${label}"), otherwise the ` +
          'route back to the mascot points at a menu item that does not exist'
      ).toContain(label);
    }
  });

  it.each(Object.keys(LOCALES))('%s uses one verb across title, buttons and setting', locale => {
    const t = LOCALES[locale];
    // Title, confirm button and the Appearance switch should read as the same
    // action; mixing "remove"/"hide"/"send away" makes one dialog look like
    // three different outcomes.
    for (const key of [
      'chat.mascot.dismiss',
      'chat.mascot.dismissTitle',
      'chat.mascot.dismissConfirm',
      'chat.mascot.dismissCancel',
      'settings.appearance.showChatMascot',
    ]) {
      expect(t[key], `${locale} is missing ${key}`).toBeTruthy();
    }
    expect(t['chat.mascot.dismissTitle']).toContain('Tiny');
    expect(t['chat.mascot.dismissConfirm']).toContain('Tiny');
    expect(t['chat.mascot.dismissCancel']).toContain('Tiny');
  });
});
