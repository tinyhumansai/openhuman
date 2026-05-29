import { describe, expect, it } from 'vitest';

import { cronToHuman } from './cronToHuman';

describe('cronToHuman', () => {
  describe('SkillsRunnerBody / DevWorkflowPanel preset expressions', () => {
    it('every 30 minutes', () => {
      expect(cronToHuman('*/30 * * * *')).toBe('Every 30 minutes');
    });

    it('every hour (minute=0)', () => {
      expect(cronToHuman('0 * * * *')).toBe('Every hour');
    });

    it('every 2 hours', () => {
      expect(cronToHuman('0 */2 * * *')).toBe('Every 2 hours');
    });

    it('every 6 hours', () => {
      expect(cronToHuman('0 */6 * * *')).toBe('Every 6 hours');
    });

    it('once daily at 09:00', () => {
      expect(cronToHuman('0 9 * * *')).toBe('Daily at 09:00');
    });
  });

  describe('generic patterns', () => {
    it('every minute', () => {
      expect(cronToHuman('*/1 * * * *')).toBe('Every minute');
    });

    it('hourly at a non-zero minute', () => {
      expect(cronToHuman('15 * * * *')).toBe('Hourly at :15');
    });

    it('every N hours with a non-zero minute offset', () => {
      expect(cronToHuman('30 */3 * * *')).toBe('Every 3 hours at :30');
    });

    it('every hour (step=1) with non-zero minute', () => {
      expect(cronToHuman('45 */1 * * *')).toBe('Every hour at :45');
    });

    it('daily at a non-rounded hour:minute', () => {
      expect(cronToHuman('30 14 * * *')).toBe('Daily at 14:30');
    });
  });

  describe('edge cases', () => {
    it('empty string', () => {
      expect(cronToHuman('')).toBe('');
    });

    it('whitespace only', () => {
      expect(cronToHuman('   ')).toBe('');
    });

    it('not a string', () => {
      // @ts-expect-error testing runtime fallthrough on bad input
      expect(cronToHuman(null)).toBe('');
      // @ts-expect-error testing runtime fallthrough on bad input
      expect(cronToHuman(undefined)).toBe('');
    });

    it('wrong number of fields falls back to raw expression', () => {
      expect(cronToHuman('* * *')).toBe('* * *');
      expect(cronToHuman('0 0 1 1 0 2026')).toBe('0 0 1 1 0 2026');
    });

    it('day-of-month constraint falls back to raw expression', () => {
      expect(cronToHuman('0 9 1 * *')).toBe('0 9 1 * *');
    });

    it('day-of-week constraint falls back to raw expression', () => {
      expect(cronToHuman('0 9 * * 1')).toBe('0 9 * * 1');
    });

    it('month constraint falls back to raw expression', () => {
      expect(cronToHuman('0 9 * 6 *')).toBe('0 9 * 6 *');
    });

    it('collapses extra whitespace before parsing', () => {
      expect(cronToHuman('  0   9   *   *   *  ')).toBe('Daily at 09:00');
    });
  });
});
