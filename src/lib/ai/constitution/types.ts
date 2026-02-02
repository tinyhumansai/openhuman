/** Core principles from the constitution */
export interface ConstitutionPrinciple {
  id: string;
  title: string;
  description: string;
}

/** Memory principles that guide what gets persisted */
export interface MemoryPrinciple {
  rule: string;
}

/** Decision framework evaluation criteria */
export interface DecisionCriterion {
  id: string;
  question: string;
}

/** Actions that the agent must never perform */
export interface ProhibitedAction {
  description: string;
}

/** Full parsed constitution */
export interface ConstitutionConfig {
  /** Raw markdown source */
  raw: string;
  /** Core behavioral principles */
  corePrinciples: ConstitutionPrinciple[];
  /** Memory formation rules */
  memoryPrinciples: MemoryPrinciple[];
  /** Pre-action evaluation criteria */
  decisionFramework: DecisionCriterion[];
  /** Hard-blocked actions */
  prohibitedActions: ProhibitedAction[];
  /** Interaction style guidelines */
  interactionGuidelines: string[];
  /** Whether this is the default constitution or user-customized */
  isDefault: boolean;
}

/** Validation result for constitution compliance */
export interface ConstitutionValidation {
  valid: boolean;
  violations: ConstitutionViolation[];
}

/** A specific constitution violation */
export interface ConstitutionViolation {
  rule: string;
  category: 'safety' | 'privacy' | 'accuracy' | 'consent' | 'reversibility';
  severity: 'error' | 'warning';
  message: string;
}
